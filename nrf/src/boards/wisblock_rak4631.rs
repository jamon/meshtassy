//! Example board template - copy this file and modify for your board
//!
//! Pin assignments and peripheral configuration for RAK4631 on Wisblock 19003 (also an nrf52840).

use super::{BoardPeripherals, LoRaPeripherals};
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::twim::Twim;
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_nrf::{bind_interrupts, pac, peripherals, rng, spim, twim, usb};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Delay;
use embedded_hal_bus::spi::ExclusiveDevice;
use static_cell::{ConstStaticCell, StaticCell};

bind_interrupts!(struct Irqs {
    TWISPI1 => twim::InterruptHandler<peripherals::TWISPI1>;
    TWISPI0 => spim::InterruptHandler<peripherals::TWISPI0>;
    RNG => rng::InterruptHandler<peripherals::RNG>;
    USBD => usb::InterruptHandler<peripherals::USBD>;
    CLOCK_POWER => usb::vbus_detect::InterruptHandler;
});

/// Pin assignments for RAK4631 on Wisblock 19003
///
/// LoRa Radio (SX126x):
/// - NSS (Chip Select): P1_10
/// - Reset: P1_06
/// - DIO1: P1_15
/// - Busy: P1_14
/// - SPI: TWISPI0 (SCK: P1_11, MISO: P1_13, MOSI: P1_12) <- uses spim interrupt
/// - I2C: TWISPI1 (SDA: P0_13, SCL: P0_14) <- uses twim interrupt
///
/// LEDs:
/// - Red: P0_XX (does not have one? or is unclear where it is)
/// - Green: P1_03
/// - Blue: P1_04
pub fn init_board(p: embassy_nrf::Peripherals) -> BoardPeripherals {
    // Initialize external high-frequency oscillator for USB
    pac::CLOCK.tasks_hfclkstart().write_value(1);
    while pac::CLOCK.events_hfclkstarted().read() != 1 {}

    // Configure LoRa radio pins
    let nss = Output::new(p.P1_10, Level::High, OutputDrive::Standard); // CS pin
    let reset = Output::new(p.P1_05, Level::High, OutputDrive::Standard); // Reset pin
    let dio1 = Input::new(p.P1_15, Pull::Down); // DIO1 pin
    let busy = Input::new(p.P1_14, Pull::None); // Busy pin

    // Configure SPI for LoRa radio
    let mut spi_config = spim::Config::default();
    spi_config.frequency = spim::Frequency::M16;
    let spi_sck = p.P1_11; // SCK pin
    let spi_miso = p.P1_13; // MISO pin
    let spi_mosi = p.P1_12; // MOSI pin
    let spim = spim::Spim::new(p.TWISPI0, Irqs, spi_sck, spi_miso, spi_mosi, spi_config);
    let spi = ExclusiveDevice::new(spim, nss, Delay);

    // Configure the I2C bus for sensors
    static RAM_BUFFER: ConstStaticCell<[u8; 16]> = ConstStaticCell::new([0; 16]);
    static I2C_BUS: StaticCell<Mutex<NoopRawMutex, Twim<'_, embassy_nrf::peripherals::TWISPI1>>> =
        StaticCell::new();
    let i2c_config = twim::Config::default();
    let i2c = Twim::new(
        p.TWISPI1,
        Irqs,
        p.P0_13,
        p.P0_14,
        i2c_config,
        RAM_BUFFER.take(),
    );
    let i2c_bus = Mutex::new(i2c);
    let i2c_bus = I2C_BUS.init(i2c_bus);

    // Configure USB driver
    let usb_driver = usb::Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs));

    // Configure RNG
    let rng = rng::Rng::new_blocking(p.RNG);

    BoardPeripherals {
        lora: LoRaPeripherals {
            spi,
            reset,
            dio1,
            busy,
        },
        leds: None,
        usb_driver,
        rng,
        i2c: Some(i2c_bus),
    }
}
