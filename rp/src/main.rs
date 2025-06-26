#![no_std]
#![no_main]

use core::u32;

use crate::usb_framer::Framer;
use defmt::*;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_rp::peripherals;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::{PubSubBehavior, PubSubChannel};
use embassy_time::Delay;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use femtopb::Message as _;
use lora_phy::iv::GenericSx126xInterfaceVariant;
use lora_phy::sx126x::{Sx1262, Sx126x, Sx126xVariant, TcxoCtrlVoltage};
use lora_phy::{mod_params::*, sx126x};
use lora_phy::{LoRa, RxMode};
use static_cell::StaticCell;

use {defmt_rtt as _, panic_probe as _};

use embassy_futures::join::join;
use embassy_futures::select::{select, Either};
use embassy_rp::usb::{Driver, Instance};
use embassy_usb::driver::{EndpointError};
use embassy_usb::{Builder, Config};

use meshtassy_net::header::HeaderFlags;
use meshtassy_net::key::ChannelKey;
use meshtassy_net::{DecodedPacket, Decrypted, Encrypted, Header, Packet};
use meshtastic_protobufs::meshtastic::{Data, FromRadio, MyNodeInfo, PortNum, ToRadio, NodeInfo, User};
mod usb_framer;

mod boards;

static PACKET_CHANNEL: PubSubChannel<CriticalSectionRawMutex, DecodedPacket, 8, 8, 1> =
    PubSubChannel::<CriticalSectionRawMutex, DecodedPacket, 8, 8, 1>::new();

static NODE_DATABASE: Mutex<
    CriticalSectionRawMutex,
    Option<meshtassy_net::node_database::NodeDatabase>,
> = Mutex::new(None);

// Packet ID counter for USB serial packets
static PACKET_ID_COUNTER: Mutex<CriticalSectionRawMutex, u32> = Mutex::new(1);

// USB static allocations for Embassy's Forever pattern
static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
static MSOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
static STATE: StaticCell<State> = StaticCell::new();

// Meshtastic LoRa parameters
const LORA_PREAMBLE_LENGTH: u16 = 16;
const LORA_SYNCWORD: u8 = 0x2B;

// Meshtastic US Default Frequency
const LORA_FREQUENCY_IN_HZ: u32 = 906_875_000;

// Meshtastic LongFast LoRa parameters
const LORA_SF: SpreadingFactor = SpreadingFactor::_11;
const LORA_BANDWIDTH: Bandwidth = Bandwidth::_250KHz;
const LORA_CODINGRATE: CodingRate = CodingRate::_4_5;

// This example task processes incoming packets from the Meshtastic radio.
// It subscribes to the PACKET_CHANNEL and handles each packet as it arrives.
#[embassy_executor::task]
async fn packet_processor_task() {
    info!("Starting packet processor task");
    let mut subscriber = PACKET_CHANNEL.subscriber().unwrap();
    loop {
        let wait_result = subscriber.next_message().await;
        let packet = match wait_result {
            embassy_sync::pubsub::WaitResult::Message(msg) => msg,
            embassy_sync::pubsub::WaitResult::Lagged(_) => {
                info!("Packet processor lagged, continuing...");
                continue;
            }
        }; // Process the received packet

        // Add or update the node in the database using the packet
        let mut db_guard = NODE_DATABASE.lock().await;
        if let Some(ref mut db) = *db_guard {
            let _success = db.add_or_update_node_from_packet(&packet);
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // Initialize board-specific peripherals
    let board = boards::init_board(p);    // USB
    let driver = board.usb_driver;

    let mut config = Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("Embassy");
    config.product = Some("USB-serial example");
    config.serial_number = Some("12345678");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    // Use static allocations for USB descriptors and buffers
    let config_descriptor = CONFIG_DESCRIPTOR.init([0; 256]);
    let bos_descriptor = BOS_DESCRIPTOR.init([0; 256]);
    let control_buf = CONTROL_BUF.init([0; 64]);
    let state = STATE.init(State::new());

    let mut builder = Builder::new(
        driver,
        config,
        config_descriptor,
        bos_descriptor,
        &mut [],
        control_buf,
    );

    let cdc = CdcAcmClass::new(&mut builder, state, 64);
    let usb = builder.build();

    // LoRa radio setup using board peripherals
    let spi = board.lora.spi;
    let reset = board.lora.reset;
    let dio1 = board.lora.dio1;
    let busy = board.lora.busy;

    // Try initializing a BME
    //TODO: throw this in an embassy task that eventually scans a given i2c bus and configs the
    //sensors
    
    if let Some(i2c_bus) = board.i2c {
        use meshtassy_telemetry::environmental_telemetry::EnvironmentData;
        use meshtassy_telemetry::TelemetrySensor;
        use meshtassy_telemetry::sensors::bme::BME;
        use meshtassy_telemetry::sensors::scd30::SCD30;
        use crate::boards::I2CBus;

        let i2c_dev1 = I2cDevice::new(i2c_bus);
        let mut bme = TelemetrySensor::<BME<'_, I2CBus>>::new(i2c_dev1);
        bme.setup().await;
        let i2c_dev2 = I2cDevice::new(i2c_bus);
        let mut scd30 = TelemetrySensor::<SCD30<'_, I2CBus>>::new(i2c_dev2);
        scd30.setup().await;
        let metrics = scd30.get_metrics().await;
        let metrics = bme.get_metrics().await;
    }

    // are we configured to use DIO2 as RF switch?  (This should be true for Sx1262)
    info!("Use dio2 as RFSwitch? {:?}", Sx1262.use_dio2_as_rfswitch());

    let config = sx126x::Config {
        chip: Sx1262,
        tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V7),
        use_dcdc: true,
        rx_boost: true,
    };    
    let iv = GenericSx126xInterfaceVariant::new(reset, dio1, busy, None, None).unwrap();
    let radio = Sx126x::new(spi, iv, config);    
    let mut lora = LoRa::with_syncword(radio, LORA_SYNCWORD, Delay)
        .await
        .unwrap();

    // Setup LEDs using board peripherals (if available)
    if let Some(leds) = board.leds {
        let mut led_red = leds.red;
        let mut led_green = leds.green;
        let mut led_blue = leds.blue;
        led_green.set_low();  // Turn on green LED (active low)
        led_blue.set_low();   // Turn on blue LED (active low)
        led_red.set_low();    // Turn on red LED (active low)
        info!("LEDs initialized");
    } else {
        info!("No LEDs available on this board");
    }// Spawn the packet processor task
    spawner.spawn(packet_processor_task()).unwrap();

    // Spawn the USB serial task
    spawner.spawn(usb_serial_task(usb, cdc)).unwrap();

    // Initialize the node databases
    initialize_node_database().await;

    info!(
        "Starting Meshtastic Radio on frequency {} Hz with syncword 0x{:02X}",
        LORA_FREQUENCY_IN_HZ, LORA_SYNCWORD
    );

    let mut receiving_buffer = [0u8; 256];

    let mdltn_params = {
        match lora.create_modulation_params(
            LORA_SF,
            LORA_BANDWIDTH,
            LORA_CODINGRATE,
            LORA_FREQUENCY_IN_HZ,
        ) {
            Ok(mp) => mp,
            Err(err) => {
                info!("Radio error = {}", err);
                return;
            }
        }
    };

    let rx_pkt_params = {
        match lora.create_rx_packet_params(
            LORA_PREAMBLE_LENGTH,
            false,
            receiving_buffer.len() as u8,
            true,
            false,
            &mdltn_params,
        ) {
            Ok(pp) => pp,
            Err(err) => {
                info!("Radio error = {}", err);
                return;
            }
        }
    };

    let _tx_pkt_params = {
        match lora.create_tx_packet_params(LORA_PREAMBLE_LENGTH, false, true, false, &mdltn_params)
        {
            Ok(pp) => pp,
            Err(err) => {
                info!("Radio error = {}", err);
                return;
            }
        }    };
    let mut rng = board.rng;
    let mut bytes = [0u8; 4];
    //rng.blocking_fill_bytes(&mut bytes); <- unavailable here and RoscRNG may be wrong
    let tx_packet_id = u32::from_le_bytes(bytes); // Create the transmission header
    let tx_header = Header {
        source: 0xDEADBEEF,
        destination: 0xFFFFFFFF,
        packet_id: tx_packet_id,
        flags: HeaderFlags {
            hop_limit: 7,
            hop_start: 7,
            want_ack: false,
            via_mqtt: false,
        },
        channel_hash: 0x08, // calculate this at some point
        next_hop: 0x00,
        relay_node: 0x00,
    };

    info!("TX Header: {}", tx_header);

    // Create and send a test message
    let mut tx_buffer = [0u8; 256];
    if let Some(packet_len) =
        create_text_message_packet(&tx_header, "Hello, world!", &[0x01u8], 1, &mut tx_buffer)
    {
        info!("Created message packet with length: {}", packet_len);

        // Test our packet decoding by processing it through handle_received_packet
        info!("Testing packet decoding with our created packet:");
        handle_received_packet(
            &tx_buffer, packet_len, 10,  // Mock SNR value
            -50, // Mock RSSI value
        );

        // match lora
        //     .prepare_for_tx(
        //         &mdltn_params,
        //         &mut tx_pkt_params,
        //         packet_len as i32,
        //         &tx_buffer[..packet_len],
        //     )
        //     .await
        // {
        //     Ok(()) => {
        //         info!("Radio prepared for TX");
        //         match lora.tx().await {
        //             Ok(()) => info!("TX DONE - Packet transmitted successfully!"),
        //             Err(err) => info!("Radio TX error: {}", err),
        //         }
        //     }
        //     Err(err) => info!("Radio prepare_for_tx error: {}", err),
        // }
    } else {
        info!("Failed to create message packet");
    }

    // RX
    match lora
        .prepare_for_rx(RxMode::Continuous, &mdltn_params, &rx_pkt_params)
        .await
    {
        Ok(()) => {}
        Err(err) => {
            info!("Radio error = {}", err);
            return;
        }
    };

    loop {
        receiving_buffer.fill(0);

        match lora.rx(&rx_pkt_params, &mut receiving_buffer).await {
            Ok((received_len, rx_pkt_status)) => {
                trace!("rx successful, len = {}, {}", received_len, rx_pkt_status);

                let received_len = received_len as usize;
                trace!("Received packet: {:02X}", &receiving_buffer[..received_len]); // decode header
                let _header = Header::from_bytes(&receiving_buffer[..16]).unwrap();
                handle_received_packet(
                    &receiving_buffer,
                    received_len,
                    rx_pkt_status.snr,
                    rx_pkt_status.rssi,
                );
            }
            Err(err) => info!("rx unsuccessful = {}", err),
        }
    }
}

fn log_packet_info(
    header: &Header,
    node_info: Option<&meshtassy_net::node_database::NodeInfo>,
    rssi: i16,
    snr: i16,
    port_name: &str,
) {
    match node_info {
        Some(source) => {
            info!(
                "\n{} ({:?}) - RSSI: {}, SNR: {} - {}",
                header, source, rssi, snr, port_name
            );
        }
        None => {
            info!(
                "\n{} - RSSI: {}, SNR: {} - {}",
                header, rssi, snr, port_name
            );
        }
    }
}

fn handle_received_packet(receiving_buffer: &[u8], received_len: usize, snr: i16, rssi: i16) {
    // Create channel key from raw bytes (1-byte key with default key + LSB replacement)
    // @TODO need to replace this with a proper key management system
    let Some(key) = ChannelKey::from_bytes(&[0x01; 1], 1) else {
        error!("✗ Failed to create channel key");
        return;
    };
    trace!("✓ Successfully created channel key for decryption");

    info!("=== Processing received packet ===");
    info!(
        "Received {} bytes, SNR: {}, RSSI: {}",
        received_len, snr, rssi
    );
    trace!("Raw packet: {:02X}", &receiving_buffer[..received_len]);

    // High Level overview of packet processing:
    // 1. Packet::<Encrypted>::from_bytes(buffer)  => Packet<Encrypted>
    // 2. .decrypt(&ChannelKey)                    => Packet<Decrypted>
    // 3. .decode()                                => DecodedPacket
    // the decoded packet is equivalent to the `Data` protobuf message, but also has the header, rssi, and snr fields

    // 1. Create encrypted packet from received bytes
    let Some(encrypted_pkt) =
        Packet::<Encrypted>::from_bytes(&receiving_buffer[..received_len], rssi as i8, snr as i8)
    else {
        warn!("✗ Failed to parse encrypted packet from bytes");
        return;
    };
    trace!(
        "✓ Successfully parsed encrypted packet: {:?}",
        encrypted_pkt
    );

    // 2. Decrypt the packet
    let Ok(decrypted_pkt) = encrypted_pkt.decrypt(&key) else {
        warn!("✗ Failed to decrypt packet");
        return;
    };
    trace!("✓ Successfully decrypted packet: {:?}", decrypted_pkt);

    // 3. Try to decode the packet into structured data
    let Ok(decoded_pkt) = decrypted_pkt.decode() else {
        warn!("✗ Failed to decode packet to structured data");
        return;
    };
    trace!(
        "✓ Successfully decoded packet to structured data {:?}",
        decoded_pkt
    );

    // Publish the decoded packet to the channel
    PACKET_CHANNEL.publish_immediate(decoded_pkt.clone());

    // Try to get the owned data for logging
    let Ok(owned_data) = decoded_pkt.data() else {
        warn!("✗ Failed to get owned data from decoded packet");
        return;
    };

    trace!("Decoded packet data: {:?}", owned_data);
    let portnum = owned_data.portnum;

    // Log the packet based on port type
    let port_name = match portnum {
        femtopb::EnumValue::Known(PortNum::TelemetryApp) => "TELEMETRY",
        femtopb::EnumValue::Known(PortNum::NodeinfoApp) => "NODEINFO",
        femtopb::EnumValue::Known(PortNum::PositionApp) => "POSITION",
        femtopb::EnumValue::Known(PortNum::NeighborinfoApp) => "NEIGHBORINFO",
        femtopb::EnumValue::Known(PortNum::TextMessageApp) => "TEXT",
        femtopb::EnumValue::Known(PortNum::RoutingApp) => "ROUTING",
        femtopb::EnumValue::Known(PortNum::TracerouteApp) => "TRACEROUTE",
        _ => "OTHER",
    };

    // Log packet with optional node info from database
    if let Ok(db_guard) = NODE_DATABASE.try_lock() {
        let node_info = db_guard
            .as_ref()
            .and_then(|db| db.get_node(decoded_pkt.header.source));

        log_packet_info(&decoded_pkt.header, node_info, rssi, snr, port_name);
    } else {
        log_packet_info(&decoded_pkt.header, None, rssi, snr, port_name);
    }
}

// temporary function just to test sending text messages
// This will be replaced with a proper Meshtastic API call in the future
fn create_text_message_packet(
    header: &Header,
    message: &str,
    key: &[u8],
    key_len: usize,
    tx_buffer: &mut [u8; 256],
) -> Option<usize> {
    use meshtassy_net::key::ChannelKey;

    // Create the data payload
    let data = Data {
        portnum: femtopb::EnumValue::Known(PortNum::TextMessageApp),
        payload: message.as_bytes(),
        want_response: false,
        dest: 0,
        source: 0,
        request_id: 0,
        reply_id: 0,
        emoji: 0,
        bitfield: Some(0),
        unknown_fields: Default::default(),
    };

    // Encode the data payload to protobuf
    let mut payload_buffer = [0u8; 240]; // Leave room for header (256 - 16)
    let buffer_len = payload_buffer.len();
    let mut slice = payload_buffer.as_mut_slice();

    if let Err(_) = data.encode(&mut slice) {
        info!("Failed to encode data");
        return None;
    }

    let encoded_payload_len = buffer_len - slice.len();
    info!("Encoded payload length: {} bytes", encoded_payload_len);
    info!(
        "Encoded payload: {:02X}",
        &payload_buffer[..encoded_payload_len]
    );

    // Create channel key
    if let Some(channel_key) = ChannelKey::from_bytes(key, key_len) {
        // Create a decrypted packet first
        let mut full_payload = [0u8; 240];
        full_payload[..encoded_payload_len].copy_from_slice(&payload_buffer[..encoded_payload_len]);
        let _decrypted_packet = Packet::<Decrypted>::new(
            header.clone(),
            0, // rssi placeholder
            0, // snr placeholder
            full_payload,
            encoded_payload_len,
        );

        // Now we need to encrypt this. For now, let's manually do the encryption
        // Copy header to output buffer
        tx_buffer[..16].copy_from_slice(&header.to_bytes());

        // Copy payload to output buffer
        tx_buffer[16..16 + encoded_payload_len]
            .copy_from_slice(&payload_buffer[..encoded_payload_len]);
        // Generate IV from header using the correct Meshtastic protocol format
        let iv = header.create_iv();

        // Encrypt in place
        match channel_key.transform(&mut tx_buffer[16..16 + encoded_payload_len], &iv) {
            Ok(()) => {
                let total_len = 16 + encoded_payload_len;
                info!("Successfully encrypted packet! Length: {} bytes", total_len);
                info!("Encrypted packet: {:02X}", &tx_buffer[..total_len]);
                Some(total_len)
            }
            Err(_) => {
                info!("Failed to encrypt packet");
                None
            }
        }
    } else {
        info!("Failed to create channel key");
        None
    }
}


struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(val: EndpointError) -> Self {
        match val {
            EndpointError::BufferOverflow => {
                info!("Buffer overflow");
                Disconnected {}
            }
            EndpointError::Disabled => Disconnected {},
        }
    }
}

// Helper function to encode and send a FromRadio packet over USB
async fn send_packet_to_usb<'d, T: Instance + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T>>,
    from_radio_packet: &FromRadio<'_>,
    buffer: &mut [u8; 256],
) -> Result<(), Disconnected> {
    // Encode the FromRadio packet
    let Some(encoded_len) = encode_from_radio_packet(from_radio_packet, buffer) else {
        info!("✗ Failed to encode FromRadio packet");
        return Err(Disconnected {});
    };

    info!("Preparing to send FromRadio packet over USB serial...");

    // Create header with magic bytes and length
    let mut header = [0u8; 4];
    header[0] = 0x94;
    header[1] = 0xc3;
    let length_bytes = (encoded_len as u16).to_be_bytes();
    header[2] = length_bytes[0];
    header[3] = length_bytes[1];

    info!("Sending packet with header: {:02X}", &header);
    class.write_packet(&header).await?;

    // Send the encoded packet data in 64-byte chunks
    info!("Sending encoded packet: {:02X}", &buffer[..encoded_len]);
    for chunk in buffer[..encoded_len].chunks(64) {
        class.write_packet(chunk).await?;
    }

    info!("FromRadio packet sent successfully");
    Ok(())
}

async fn packet_forwarder<'d, T: Instance + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T>>,
) -> Result<(), Disconnected> {
    let mut subscriber = PACKET_CHANNEL.subscriber().unwrap();

    // Simple state machine for packet reception
    info!("Waiting for command packet from USB serial...");

    let mut buf = [0u8; 64]; // USB packet buffer
    let mut packet_buffer = [0u8; 512]; // Buffer to store the complete packet
    let mut framer = Framer::new();

    loop {
        // Use embassy-futures select function to handle both USB reads and subscriber messages
        match select(class.read_packet(&mut buf), subscriber.next_message()).await {
            Either::First(_) => {
                // Handle USB CDC ACM read
                // @TODO two requests in single packet could fail, this should be a while loop
                if let Some(packet) = framer.push_bytes(&buf) {
                    let packet_len = packet.len().min(packet_buffer.len());
                    packet_buffer[..packet_len].copy_from_slice(&packet[..packet_len]);
                    info!(
                        "Received command packet: {:02X}",
                        &packet_buffer[..packet_len]
                    );

                    info!("Received command packet: {:02X}", packet);
                    let Ok(decoded_packet) = ToRadio::decode(&packet) else {
                        info!("✗ Failed to decode ToRadio packet");
                        return Err(Disconnected {});
                    };
                    info!(
                        "✓ Successfully decoded ToRadio packet: {:?}",
                        decoded_packet
                    );

                    // Declare encoded_buffer outside the match statement so it can be used in all arms
                    let mut encoded_buffer = [0u8; 256];

                    match decoded_packet.payload_variant {
                        Some(meshtastic_protobufs::meshtastic::to_radio::PayloadVariant::WantConfigId(config_id)) => {
                            info!("Client requesting config with ID: {}", config_id);
                            
                            // Send MyNodeInfo packet
                            let packet_id = get_next_packet_id().await;
                            let from_radio_packet = create_my_node_info_packet(packet_id);
                            send_packet_to_usb(class, &from_radio_packet, &mut encoded_buffer).await?;

                            // Send NodeInfo packet for our own node
                            let packet_id = get_next_packet_id().await;
                            let from_radio_packet = create_node_info_packet(packet_id);
                            send_packet_to_usb(class, &from_radio_packet, &mut encoded_buffer).await?;

                            // Send NodeInfo packets for all nodes in the database
                            if let Ok(db_guard) = NODE_DATABASE.try_lock() {
                                if let Some(ref database) = *db_guard {
                                    let node_count = database.get_nodes().count();
                                    info!("Sending NodeInfo for {} nodes from database", node_count);
                                    
                                    for node in database.get_nodes() {
                                        // Skip our own node (already sent above)
                                        if node.num != 0xDEADBEEF {
                                            let packet_id = get_next_packet_id().await;
                                            let from_radio_packet = create_node_info_packet_from_db(packet_id, node);
                                            send_packet_to_usb(class, &from_radio_packet, &mut encoded_buffer).await?;
                                        }
                                    }
                                }
                            }

                            // Send Config packet  
                            let packet_id = get_next_packet_id().await;
                            let from_radio_packet = create_config_packet(packet_id);
                            send_packet_to_usb(class, &from_radio_packet, &mut encoded_buffer).await?;

                            // Send ModuleConfig packet
                            let packet_id = get_next_packet_id().await;
                            let from_radio_packet = create_module_config_packet(packet_id);
                            send_packet_to_usb(class, &from_radio_packet, &mut encoded_buffer).await?;

                            // Send Channel packet
                            let packet_id = get_next_packet_id().await;
                            let from_radio_packet = create_channel_packet(packet_id);
                            send_packet_to_usb(class, &from_radio_packet, &mut encoded_buffer).await?;

                            // Send ConfigComplete packet
                            let packet_id = get_next_packet_id().await;
                            let from_radio_packet = create_config_complete_packet(packet_id, config_id);
                            send_packet_to_usb(class, &from_radio_packet, &mut encoded_buffer).await?;

                        },
                        Some(meshtastic_protobufs::meshtastic::to_radio::PayloadVariant::Heartbeat(_)) => {
                            info!("Received heartbeat request - connection kept alive");
                            // Heartbeat requests typically don't require a response
                            // The device just acknowledges by staying awake and continuing the connection
                        },
                        _ => {
                            info!("Received unsupported ToRadio payload variant");
                            continue;
                        }
                    }
                } else {
                    info!("Invalid command packet received");
                    continue;
                }
            }
            Either::Second(wait_result) => {
                // Handle subscriber messages
                let packet = match wait_result {
                    embassy_sync::pubsub::WaitResult::Message(msg) => msg,
                    embassy_sync::pubsub::WaitResult::Lagged(_) => {
                        let lag_msg = b"[PACKET FORWARDER LAGGED]\n";
                        let _ = class.write_packet(lag_msg).await;
                        continue;
                    }
                };

                // Check packet type and forward real-time updates to USB client
                match packet.port_num() {
                    femtopb::EnumValue::Known(PortNum::NodeinfoApp) => {
                        info!("Received NodeInfo packet from node {}, forwarding to client", packet.header.source);
                        
                        // Try to get the node from the database and send a NodeInfo packet
                        if let Ok(db_guard) = NODE_DATABASE.try_lock() {
                            if let Some(ref database) = *db_guard {
                                if let Some(node) = database.get_node(packet.header.source) {
                                    // Generate a unique packet ID for this real-time NodeInfo update
                                    let packet_id = get_next_packet_id().await;
                                    let from_radio_packet = create_node_info_packet_from_db(packet_id, node);
                                    
                                    let mut encoded_buffer = [0u8; 256];
                                    if let Err(_) = send_packet_to_usb(class, &from_radio_packet, &mut encoded_buffer).await {
                                        info!("Failed to send NodeInfo packet to USB");
                                    } else {
                                        info!("Successfully sent NodeInfo packet for node {} to USB", packet.header.source);
                                    }
                                }
                            }
                        }
                    },
                    femtopb::EnumValue::Known(PortNum::PositionApp) |
                    femtopb::EnumValue::Known(PortNum::TelemetryApp) |
                    femtopb::EnumValue::Known(PortNum::TextMessageApp) |
                    femtopb::EnumValue::Known(PortNum::RoutingApp) |
                    femtopb::EnumValue::Known(PortNum::TracerouteApp) |
                    femtopb::EnumValue::Known(PortNum::NeighborinfoApp) => {
                        let port_name = match packet.port_num() {
                            femtopb::EnumValue::Known(PortNum::PositionApp) => "Position",
                            femtopb::EnumValue::Known(PortNum::TelemetryApp) => "Telemetry",
                            femtopb::EnumValue::Known(PortNum::TextMessageApp) => "Text Message",
                            femtopb::EnumValue::Known(PortNum::RoutingApp) => "Routing",
                            femtopb::EnumValue::Known(PortNum::TracerouteApp) => "Traceroute",
                            femtopb::EnumValue::Known(PortNum::NeighborinfoApp) => "Neighbor Info",
                            _ => "Unknown", // This should never happen due to the match above
                        };
                        
                        info!("Received {} packet from node {}, forwarding to client", port_name, packet.header.source);
                        
                        // Create a generic MeshPacket FromRadio packet with the received data
                        let packet_id = get_next_packet_id().await;
                        if let Some(from_radio_packet) = create_mesh_packet_from_data(packet_id, &packet) {
                            let mut encoded_buffer = [0u8; 256];
                            if let Err(_) = send_packet_to_usb(class, &from_radio_packet, &mut encoded_buffer).await {
                                info!("Failed to send {} packet to USB", port_name);
                            } else {
                                info!("Successfully sent {} packet for node {} to USB", port_name, packet.header.source);
                            }
                        } else {
                            info!("Failed to create {} packet for node {}, skipping", port_name, packet.header.source);
                        }
                    },
                    _ => {
                        // For other packet types, we don't forward them as FromRadio packets
                    }
                }
            }
        }
    }
}

// USB Serial task - handles USB CDC ACM communication
// This task will manage the USB serial interface for debugging and communication
#[embassy_executor::task]
async fn usb_serial_task(
    mut usb: embassy_usb::UsbDevice<
        'static,
        Driver<'static, peripherals::USB>,
    >,
    mut cdc: CdcAcmClass<'static, Driver<'static, peripherals::USB>>,
) {
    info!("Starting USB serial task");
    let usb_fut = usb.run();

    let packet_forwarder_fut = async {
        info!("Waiting for USB connection...");
        loop {
            cdc.wait_connection().await;
            info!("USB Connected - Starting packet forwarding");
            let _ = packet_forwarder(&mut cdc).await;
            info!("USB Disconnected - Stopping packet forwarding");
        }
    };
    join(usb_fut, packet_forwarder_fut).await;
}

async fn initialize_node_database() {
    // Initialize the node database
    let mut database_guard = NODE_DATABASE.lock().await;
    *database_guard = Some(meshtassy_net::node_database::NodeDatabase::new());
    info!("Node database initialized");
}

/// Get the next packet ID for USB serial communication
/// This ensures we never overflow by wrapping at a reasonable value
async fn get_next_packet_id() -> u32 {
    let mut counter_guard = PACKET_ID_COUNTER.lock().await;
    let current_id = *counter_guard;
    // Wrap at 0x7FFFFFFF to avoid potential issues with large values
    // and to leave room for expansion
    *counter_guard = if current_id >= 0x7FFFFFFF { 1 } else { current_id + 1 };
    current_id
}

/// Create a FromRadio packet containing MyNodeInfo with hardcoded values
/// This demonstrates how to construct a basic MyNodeInfo packet for device identification
fn create_my_node_info_packet(packet_id: u32) -> FromRadio<'static> {
    FromRadio {
        id: packet_id,
        payload_variant: Some(
            meshtastic_protobufs::meshtastic::from_radio::PayloadVariant::MyInfo(MyNodeInfo {
                my_node_num: 0xDEADBEEF, // Hardcoded node number - should be unique device ID
                reboot_count: 42,        // Number of reboots (hardcoded for demo)
                min_app_version: 30200,  // Minimum app version (3.2.0)
                device_id: b"EMBASSY_RP2040", // 16-byte device identifier
                pio_env: "embassy_rp2040", // Platform environment name
                unknown_fields: Default::default(),
            }),
        ),
        unknown_fields: Default::default(),
    }
}

/// Create a FromRadio packet containing NodeInfo for our own node
fn create_node_info_packet(packet_id: u32) -> FromRadio<'static> {
    use meshtastic_protobufs::meshtastic::{config, HardwareModel};
    
    let user = User {
        id: "!deadbeef",  // Use the same node ID as in MyNodeInfo
        long_name: "Embassy NRF52",
        short_name: "ENRF",
        macaddr: &[],  // Deprecated field
        hw_model: femtopb::EnumValue::Known(HardwareModel::Unset),
        is_licensed: false,
        role: femtopb::EnumValue::Known(config::device_config::Role::Client),
        public_key: &[],  // No public key for now
        is_unmessagable: Some(false),
        unknown_fields: Default::default(),
    };

    let node_info = NodeInfo {
        num: 0xDEADBEEF,  // Same as MyNodeInfo.my_node_num
        user: Some(user),
        position: None,  // No position info for now
        snr: 0.0,
        last_heard: 0,  // Current timestamp would be better
        device_metrics: None,
        channel: 0,
        via_mqtt: false,
        hops_away: Some(0),  // We are 0 hops from ourselves
        is_favorite: false,
        is_ignored: false,
        is_key_manually_verified: false,
        unknown_fields: Default::default(),
    };

    FromRadio {
        id: packet_id,
        payload_variant: Some(
            meshtastic_protobufs::meshtastic::from_radio::PayloadVariant::NodeInfo(node_info),
        ),
        unknown_fields: Default::default(),
    }
}

/// Create a FromRadio packet containing ConfigComplete with hardcoded values
/// This demonstrates how to construct a basic ConfigComplete packet for device identification
fn create_config_complete_packet(packet_id: u32, config_complete_id: u32) -> FromRadio<'static> {
    FromRadio {
        id: packet_id,
        payload_variant: Some(
            meshtastic_protobufs::meshtastic::from_radio::PayloadVariant::ConfigCompleteId(
                config_complete_id,
            ),
        ),
        unknown_fields: Default::default(),
    }
}

/// Create a FromRadio packet containing Config with minimal placeholder data
fn create_config_packet(packet_id: u32) -> FromRadio<'static> {
    use meshtastic_protobufs::meshtastic::{Config, config};
    
    let device_config = config::DeviceConfig {
        role: femtopb::EnumValue::Known(config::device_config::Role::Client),
        serial_enabled: false,
        button_gpio: 0,
        buzzer_gpio: 0,
        rebroadcast_mode: femtopb::EnumValue::Known(config::device_config::RebroadcastMode::All),
        node_info_broadcast_secs: 900,
        double_tap_as_button_press: false,
        is_managed: false,
        disable_triple_click: false,
        tzdef: "",
        led_heartbeat_disabled: false,
        unknown_fields: Default::default(),
    };

    let config = Config {
        payload_variant: Some(config::PayloadVariant::Device(device_config)),
        unknown_fields: Default::default(),
    };

    FromRadio {
        id: packet_id,
        payload_variant: Some(
            meshtastic_protobufs::meshtastic::from_radio::PayloadVariant::Config(config),
        ),
        unknown_fields: Default::default(),
    }
}

/// Create a FromRadio packet containing ModuleConfig with minimal placeholder data
fn create_module_config_packet(packet_id: u32) -> FromRadio<'static> {
    use meshtastic_protobufs::meshtastic::{ModuleConfig, module_config};
    
    let mqtt_config = module_config::MqttConfig {
        enabled: false,
        address: "",
        username: "",
        password: "",
        encryption_enabled: false,
        json_enabled: false,
        tls_enabled: false,
        root: "",
        proxy_to_client_enabled: false,
        map_reporting_enabled: false,
        map_report_settings: None,
        unknown_fields: Default::default(),
    };

    let module_config = ModuleConfig {
        payload_variant: Some(module_config::PayloadVariant::Mqtt(mqtt_config)),
        unknown_fields: Default::default(),
    };

    FromRadio {
        id: packet_id,
        payload_variant: Some(
            meshtastic_protobufs::meshtastic::from_radio::PayloadVariant::ModuleConfig(module_config),
        ),
        unknown_fields: Default::default(),
    }
}

/// Create a FromRadio packet containing Channel with minimal placeholder data
fn create_channel_packet(packet_id: u32) -> FromRadio<'static> {
    use meshtastic_protobufs::meshtastic::{Channel, ChannelSettings, channel};
    
    let channel_settings = ChannelSettings {
        channel_num: 0, // Deprecated but required
        psk: &[0x01], // Default AES key
        name: "LongFast",
        id: 0,
        uplink_enabled: false,
        downlink_enabled: false,
        module_settings: None,
        unknown_fields: Default::default(),
    };

    let channel = Channel {
        index: 0,
        settings: Some(channel_settings),
        role: femtopb::EnumValue::Known(channel::Role::Primary),
        unknown_fields: Default::default(),
    };

    FromRadio {
        id: packet_id,
        payload_variant: Some(
            meshtastic_protobufs::meshtastic::from_radio::PayloadVariant::Channel(channel),
        ),
        unknown_fields: Default::default(),
    }
}


/// Encode a FromRadio packet to bytes for transmission over serial/BLE/etc
fn encode_from_radio_packet(packet: &FromRadio, buffer: &mut [u8]) -> Option<usize> {
    let buffer_len = buffer.len();
    let mut slice = &mut buffer[..];

    match packet.encode(&mut slice) {
        Ok(_) => {
            let encoded_len = buffer_len - slice.len();
            Some(encoded_len)
        }
        Err(_) => None,
    }
}

/// Create a FromRadio packet containing NodeInfo from our internal node database
fn create_node_info_packet_from_db(packet_id: u32, node: &meshtassy_net::node_database::NodeInfo) -> FromRadio {
    use meshtastic_protobufs::meshtastic::{User, Position, DeviceMetrics, position};
    
    // Convert user information from node database to protobuf User
    let user = node.user.as_ref().map(|db_user| {
        User {
            id: "",  // ID field is deprecated in newer versions
            long_name: db_user.long_name.as_str(),
            short_name: db_user.short_name.as_str(),
            macaddr: &[],  // Deprecated field
            hw_model: db_user.hw_model,
            is_licensed: db_user.is_licensed,
            role: db_user.role,
            public_key: &[],  // No public key for now
            is_unmessagable: Some(false),
            unknown_fields: Default::default(),
        }
    });

    // Convert position information from node database to protobuf Position
    let position = node.position.as_ref().map(|db_pos| {
        Position {
            latitude_i: Some(db_pos.latitude_i),
            longitude_i: Some(db_pos.longitude_i),
            altitude: Some(db_pos.altitude),
            time: db_pos.time,
            location_source: db_pos.location_source,
            altitude_source: femtopb::EnumValue::Known(position::AltSource::AltUnset),
            timestamp: 0,
            timestamp_millis_adjust: 0,
            altitude_hae: None,
            altitude_geoidal_separation: None,
            pdop: 0,
            hdop: 0,
            vdop: 0,
            gps_accuracy: 0,
            ground_speed: None,
            ground_track: None,
            fix_quality: 0,
            fix_type: 0,
            sats_in_view: 0,
            sensor_id: 0,
            next_update: 0,
            seq_number: 0,
            precision_bits: 0,
            unknown_fields: Default::default(),
        }
    });

    // Convert device metrics from node database to protobuf DeviceMetrics
    let device_metrics = node.device_metrics.as_ref().map(|db_metrics| {
        DeviceMetrics {
            battery_level: Some(db_metrics.battery_level),
            voltage: Some(db_metrics.voltage),
            channel_utilization: Some(db_metrics.channel_utilization),
            air_util_tx: Some(db_metrics.air_util_tx),
            uptime_seconds: Some(db_metrics.uptime_seconds),
            unknown_fields: Default::default(),
        }
    });

    let node_info = NodeInfo {
        num: node.num,
        user,
        position,
        snr: node.snr,
        last_heard: node.last_heard,
        device_metrics,
        channel: 0,
        via_mqtt: false,
        hops_away: Some(1),  // Other nodes are at least 1 hop away
        is_favorite: false,
        is_ignored: false,
        is_key_manually_verified: false,
        unknown_fields: Default::default(),
    };

    FromRadio {
        id: packet_id,
        payload_variant: Some(
            meshtastic_protobufs::meshtastic::from_radio::PayloadVariant::NodeInfo(node_info),
        ),
        unknown_fields: Default::default(),
    }
}

/// Create a generic FromRadio packet containing a MeshPacket for any supported packet type
/// This function handles all packet types that should be forwarded as FromRadio::Packet
fn create_mesh_packet_from_data(packet_id: u32, packet: &DecodedPacket) -> Option<FromRadio> {
    use meshtastic_protobufs::meshtastic::{MeshPacket, mesh_packet};
    
    // Get the owned data from the received packet
    let Ok(owned_data) = packet.data() else {
        return None;
    };

    // Check if this packet type should be forwarded as a FromRadio::Packet
    let should_forward = matches!(packet.port_num(), 
        femtopb::EnumValue::Known(PortNum::PositionApp) |
        femtopb::EnumValue::Known(PortNum::TelemetryApp) |
        femtopb::EnumValue::Known(PortNum::TextMessageApp) |
        femtopb::EnumValue::Known(PortNum::RoutingApp) |
        femtopb::EnumValue::Known(PortNum::TracerouteApp) |
        femtopb::EnumValue::Known(PortNum::NeighborinfoApp)
    );

    if !should_forward {
        return None;
    }

    // Create a generic MeshPacket containing the received data
    let mesh_packet = MeshPacket {
        from: packet.header.source,
        to: packet.header.destination,
        channel: 0,
        id: packet.header.packet_id,
        rx_time: 0, // Current time would be better
        rx_snr: packet.snr as f32,
        hop_limit: packet.header.flags.hop_limit as u32,
        want_ack: packet.header.flags.want_ack,
        priority: femtopb::EnumValue::Known(mesh_packet::Priority::Unset),
        rx_rssi: packet.rssi as i32,
        via_mqtt: packet.header.flags.via_mqtt,
        hop_start: packet.header.flags.hop_start as u32,
        public_key: &[],
        pki_encrypted: false,
        next_hop: packet.header.next_hop as u32,
        relay_node: packet.header.relay_node as u32,
        tx_after: 0,
        payload_variant: Some(mesh_packet::PayloadVariant::Decoded(meshtastic_protobufs::meshtastic::Data {
            portnum: packet.port_num(),
            payload: &owned_data.payload[..owned_data.payload_len],
            want_response: false,
            dest: packet.header.destination,
            source: packet.header.source,
            request_id: 0,
            reply_id: 0,
            emoji: 0,
            bitfield: None,
            unknown_fields: Default::default(),
        })),
        #[allow(deprecated)]
        delayed: femtopb::EnumValue::Known(mesh_packet::Delayed::NoDelay),
        unknown_fields: Default::default(),
    };

    Some(FromRadio {
        id: packet_id,
        payload_variant: Some(
            meshtastic_protobufs::meshtastic::from_radio::PayloadVariant::Packet(mesh_packet),
        ),
        unknown_fields: Default::default(),
    })
}
