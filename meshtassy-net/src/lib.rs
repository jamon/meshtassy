#![cfg_attr(not(feature = "std"), no_std)]

use base64::engine::general_purpose;
use base64::Engine;
use core::prelude::v1::*;
use femtopb::Message as _;

#[cfg(feature = "defmt")]
use defmt::Format;

// Re-export commonly used types
pub use meshtastic_protobufs::meshtastic::PortNum;

// Channel hash generation utilities
pub mod channel;

// packet header types
pub mod header;
pub use header::Header;

// key management
pub mod key;
use crate::key::ChannelKey;

// Node database for storing device information
pub mod node_database;

/// Marker types to distinguish between encrypted and decrypted packet states
#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Encrypted;

#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Decrypted;

/// Marker type for a packet with decoded payload
#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Decoded;

/// Owned Data payload that doesn't depend on zero-copy lifetimes
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct OwnedData {
    pub portnum: femtopb::EnumValue<meshtastic_protobufs::meshtastic::PortNum>,
    pub payload: [u8; 240], // Maximum payload size is 240 bytes
    pub payload_len: usize,
    pub want_response: bool,
    pub dest: u32,
    pub source: u32,
    pub request_id: u32,
    pub reply_id: u32,
    pub emoji: u32,
}

impl OwnedData {
    /// Convert from protobuf Data to owned data
    pub fn from_protobuf(data: &meshtastic_protobufs::meshtastic::Data) -> Self {
        #[cfg(feature = "defmt")]
        defmt::trace!("Converting protobuf Data to OwnedData");

        let mut new_payload = [0u8; 240];
        let copy_len = data.payload.len().min(240);
        new_payload[..copy_len].copy_from_slice(&data.payload[..copy_len]);


        Self {
            portnum: data.portnum,
            payload: new_payload,
            payload_len: copy_len,
            want_response: data.want_response,
            dest: data.dest,
            source: data.source,
            request_id: data.request_id,
            reply_id: data.reply_id,
            emoji: data.emoji,
        }
    }
}

/// A packet in various states of processing
#[derive(Clone)]
pub struct Packet<S> {
    pub header: Header,
    pub rssi: i8,
    pub snr: i8,
    pub payload: [u8; 240],
    pub payload_len: usize,
    pub _marker: core::marker::PhantomData<S>,
}

/// A decoded packet with structured data
#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DecodedPacket {
    pub header: Header,
    pub rssi: i8,
    pub snr: i8,
    pub data: OwnedData,
}

#[cfg(feature = "defmt")]
impl<S> defmt::Format for Packet<S> {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(
            fmt,
            "Packet {{ header: {}, rssi: {}, snr: {}, payload_len: {} }}",
            self.header,
            self.rssi,
            self.snr,
            self.payload_len
        );
    }
}

impl<S> Packet<S> {
    /// Create a new packet with the given data
    pub fn new(
        header: Header,
        rssi: i8,
        snr: i8,
        payload: [u8; 240],
        payload_len: usize,
    ) -> Self {
        Self {
            header,
            rssi,
            snr,
            payload,
            payload_len,
            _marker: core::marker::PhantomData,
        }
    }
}

impl Packet<Encrypted> {    
    /// Create an encrypted packet from raw bytes
    /// The buffer should contain a 16-byte header followed by encrypted payload
    pub fn from_bytes(buffer: &[u8], rssi: i8, snr: i8) -> Option<Self> {
        #[cfg(feature = "defmt")]
        defmt::trace!("Creating encrypted packet from {} bytes (RSSI: {}, SNR: {})", buffer.len(), rssi, snr);
        
        if buffer.len() < 16 {
            #[cfg(feature = "defmt")]
            defmt::error!("Buffer too small: {} bytes (minimum 16 required)", buffer.len());
            return None;
        }

        #[cfg(feature = "defmt")]
        defmt::trace!("Raw header bytes: {:02X}", &buffer[..16]);

        let header = Header::from_bytes(&buffer[..16])?;
        
        #[cfg(feature = "defmt")]
        defmt::trace!("Parsed header: {}", header);
        
        let mut payload = [0u8; 240];
        let payload_len = (buffer.len() - 16).min(240);
        payload[..payload_len].copy_from_slice(&buffer[16..16 + payload_len]);

        #[cfg(feature = "defmt")]
        defmt::trace!("Payload: {} bytes, encrypted: {:02X}", payload_len, &payload[..payload_len]);

        Some(Self {
            header,
            rssi,
            snr,
            payload,
            payload_len,
            _marker: core::marker::PhantomData,
        })
    }

    /// Decrypts the packet payload using the provided key
    /// Returns a DecryptedPacket on success, or None if decryption fails
    /// Consumes the original encrypted packet
    pub fn decrypt(self, key: &ChannelKey) -> Result<Packet<Decrypted>, ()> {
        #[cfg(feature = "defmt")]
        defmt::trace!(
            "Starting decryption process - Header: {:?}, Encrypted payload length: {}, Encrypted payload: {:02X}",
            self.header,
            self.payload_len,
            &self.payload[..self.payload_len]
        );

        // Create IV from header using the correct Meshtastic protocol format
        let iv = self.header.create_iv();
        
        #[cfg(feature = "defmt")]
        defmt::trace!("Generated IV: {:02X}, Using {:?}", iv, key);

        // Copy payload for decryption
        let mut decrypted_payload = [0u8; 240];
        decrypted_payload[..self.payload_len].copy_from_slice(&self.payload[..self.payload_len]);

        // Transform (decrypt) the payload in place
        match key.transform(&mut decrypted_payload[..self.payload_len], &iv) {
            Ok(()) => {
                #[cfg(feature = "defmt")]
                defmt::trace!("Decrypted payload: {:02X}", decrypted_payload[..self.payload_len]);
                
                Ok(Packet {
                    header: self.header,
                    rssi: self.rssi,
                    snr: self.snr,
                    payload: decrypted_payload,
                    payload_len: self.payload_len,
                    _marker: core::marker::PhantomData,
                })
            }            Err(_e) => {
                #[cfg(feature = "defmt")]
                defmt::error!("Decryption failed with error: {:?}", _e);
                Err(())
            }
        }
    }
}

impl Packet<Decrypted> {
    /// Decode the payload into structured data
    pub fn decode(self) -> Result<DecodedPacket, ()> {
        #[cfg(feature = "defmt")]
        defmt::trace!("Starting packet decode process");
        
        if self.payload_len == 0 {
            #[cfg(feature = "defmt")]
            defmt::error!("Cannot decode packet: payload is empty");
            return Err(());
        }

        #[cfg(feature = "defmt")]
        defmt::trace!(
            "Attempting to decode protobuf Data from {} bytes, full payload: {:02X}",
            self.payload_len,
            &self.payload[..self.payload_len]
        );
        let Ok(data) = meshtastic_protobufs::meshtastic::Data::decode(&self.payload[..self.payload_len]) else {
            #[cfg(feature = "defmt")]
            {
                defmt::error!("Failed to decode protobuf Data from payload");
                defmt::error!("Payload data: {:02X}", &self.payload[..self.payload_len]);
            }
            return Err(());
        };
        
        #[cfg(feature = "defmt")]
        {
            defmt::trace!(
                "Successfully decoded protobuf Data - portnum: {:?}, payload length: {}, want_response: {}, dest: 0x{:08X}, source: 0x{:08X}, request_id: {}, reply_id: {}, payload: {:02X}",
                data.portnum,
                data.payload.len(),
                data.want_response,
                data.dest,
                data.source,
                data.request_id,
                data.reply_id,
                if data.payload.len() > 0 { &data.payload[..data.payload.len().min(8)] } else { &[] }
            );
        }
        
        // Convert protobuf data to owned data
        let owned_data = OwnedData::from_protobuf(&data);
        
        #[cfg(feature = "defmt")]
        {
            defmt::trace!("Converted to OwnedData, re-encoding into packet format");
            defmt::trace!(" OwnedData {:?}", owned_data);
        }

        Ok(DecodedPacket {
            header: self.header,
            rssi: self.rssi,
            snr: self.snr,
            data: owned_data,
        })
    }
}

impl DecodedPacket {
    /// Get the port number from the decoded packet data
    pub fn port_num(&self) -> femtopb::EnumValue<meshtastic_protobufs::meshtastic::PortNum> {
        self.data.portnum
    }
    
    /// Get the payload data from the decoded packet
    pub fn payload_data(&self) -> &[u8] {
        &self.data.payload[..self.data.payload_len]
    }
    
    /// Get a reference to the owned data
    pub fn data(&self) -> Result<&OwnedData, ()> {
        Ok(&self.data)
    }
}

/// Errors that can occur during cryptographic operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub enum CryptoError {
    /// Invalid base64 encoding in key
    InvalidBase64,
    /// Decoded key is empty
    EmptyKey,
    /// Input data is empty
    EmptyData,
    /// Packet is too small or malformed
    InvalidPacket,
    /// Output buffer is too small
    BufferTooSmall,
}

/// Decodes a base64 key and zero-pads or truncates it to 32 bytes
pub fn parse_key(base64_key: &str) -> Result<[u8; 32], CryptoError> {
    let mut key_bytes: [u8; 32] = [0u8; 32];
    let mut decoded = [0u8; 64];

    match general_purpose::STANDARD.decode_slice(base64_key, &mut decoded) {
        Ok(len) => {
            if len == 0 {
                return Err(CryptoError::EmptyKey);
            }
            let copy_len = len.min(32);
            key_bytes[..copy_len].copy_from_slice(&decoded[..copy_len]);
            Ok(key_bytes)
        }
        Err(_) => Err(CryptoError::InvalidBase64),
    }
}

/// Compute a channel hash using the djb2 algorithm
pub fn channel_hash(channel_name: &str) -> u32 {
    let mut hash: u32 = 5381;

    for byte in channel_name.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u32);
    }    hash
}

/// Debug helper functions for packet analysis
impl<S> Packet<S> {

}


/// Helper function to validate packet structure
pub fn validate_packet_structure(data: &[u8]) -> Result<(), &'static str> {
    #[cfg(feature = "defmt")]
    defmt::info!("Validating packet structure for {} bytes", data.len());
    
    if data.len() < 16 {
        #[cfg(feature = "defmt")]
        defmt::error!("Packet too small: {} bytes (minimum 16)", data.len());
        return Err("Packet too small");
    }
    
    if data.len() > 256 {
        #[cfg(feature = "defmt")]
        defmt::warn!("Packet larger than expected: {} bytes (maximum ~256)", data.len());
    }
    
    // Check if header fields make sense
    let destination = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let source = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let packet_id = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    
    #[cfg(feature = "defmt")]
    {
        defmt::info!("Header validation:");
        defmt::info!("  Destination: 0x{:08X}", destination);
        defmt::info!("  Source: 0x{:08X}", source);
        defmt::info!("  Packet ID: 0x{:08X}", packet_id);
    }
    
    // Basic sanity checks
    if source == 0 && destination == 0 && packet_id == 0 {
        #[cfg(feature = "defmt")]
        defmt::warn!("All header fields are zero - this might be invalid");
    }
    
    #[cfg(feature = "defmt")]
    defmt::info!("Packet structure validation passed");
    
    Ok(())
}

/// Helper function to create a test key for debugging
pub fn create_debug_key() -> ChannelKey {
    // Use the default Meshtastic key for testing
    ChannelKey::from_bytes(&[], 0).unwrap()
}
