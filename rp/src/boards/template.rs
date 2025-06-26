//! Example board template - copy this file and modify for your board
//! 
//! Pin assignments and peripheral configuration for [YOUR BOARD NAME].

use super::{BoardPeripherals, LoRaPeripherals, LedPeripherals};
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::{bind_interrupts, pac, peripherals, rng, spim, usb};
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_time::Delay;
use embedded_hal_bus::spi::ExclusiveDevice;

bind_interrupts!(struct Irqs {
    TWISPI0 => spim::InterruptHandler<peripherals::TWISPI0>;
    RNG => rng::InterruptHandler<peripherals::RNG>;
    USBD => usb::InterruptHandler<peripherals::USBD>;
    CLOCK_POWER => usb::vbus_detect::InterruptHandler;
});

/// Pin assignments for [YOUR BOARD NAME]
/// 
/// TODO: Document your board's pin assignments here
/// 
/// LoRa Radio (SX126x):
/// - NSS (Chip Select): P0_XX
/// - Reset: P0_XX
/// - DIO1: P0_XX
/// - Busy: P0_XX
/// - SPI: TWISPI0 (SCK: P1_XX, MISO: P1_XX, MOSI: P1_XX)
///
/// LEDs:
/// - Red: P0_XX
/// - Green: P0_XX
/// - Blue: P0_XX
pub fn init_board(p: embassy_nrf::Peripherals) -> BoardPeripherals {
    // Initialize external high-frequency oscillator for USB
    pac::CLOCK.tasks_hfclkstart().write_value(1);
    while pac::CLOCK.events_hfclkstarted().read() != 1 {}

    // Configure LoRa radio pins
    // TODO: Update these pin assignments for your board
    let nss = Output::new(p.P0_04, Level::High, OutputDrive::Standard);    // CS pin
    let reset = Output::new(p.P0_28, Level::High, OutputDrive::Standard);  // Reset pin
    let dio1 = Input::new(p.P0_03, Pull::Down);                           // DIO1 pin
    let busy = Input::new(p.P0_29, Pull::None);                           // Busy pin

    // Configure SPI for LoRa radio
    // TODO: Update these pin assignments for your board
    let mut spi_config = spim::Config::default();
    spi_config.frequency = spim::Frequency::M16;
    let spi_sck = p.P1_13;   // SCK pin
    let spi_miso = p.P1_14;  // MISO pin
    let spi_mosi = p.P1_15;  // MOSI pin
    let spim = spim::Spim::new(p.TWISPI0, Irqs, spi_sck, spi_miso, spi_mosi, spi_config);
    let spi = ExclusiveDevice::new(spim, nss, Delay);

    // Configure LEDs
    // TODO: Update these pin assignments for your board
    // Note: Adjust Level::High/Low based on whether your LEDs are active high or low
    let led_red = Output::new(p.P0_26, Level::High, OutputDrive::Standard);
    let led_green = Output::new(p.P0_30, Level::High, OutputDrive::Standard);
    let led_blue = Output::new(p.P0_06, Level::High, OutputDrive::Standard);

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
        leds: LedPeripherals {
            red: led_red,
            green: led_green,
            blue: led_blue,
        },
        usb_driver,
        rng,
    }
}
