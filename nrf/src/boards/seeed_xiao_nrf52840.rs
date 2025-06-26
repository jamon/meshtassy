//! Seeed Xiao nRF52840 board-specific configuration
//!
//! Pin assignments and peripheral configuration for the Seeed Studio
//! XIAO nRF52840 development board.

use super::{BoardPeripherals, LedPeripherals, LoRaPeripherals};
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_nrf::{bind_interrupts, pac, peripherals, rng, spim, usb};
use embassy_time::Delay;
use embedded_hal_bus::spi::ExclusiveDevice;

bind_interrupts!(struct Irqs {
    TWISPI0 => spim::InterruptHandler<peripherals::TWISPI0>;
    RNG => rng::InterruptHandler<peripherals::RNG>;
    USBD => usb::InterruptHandler<peripherals::USBD>;
    CLOCK_POWER => usb::vbus_detect::InterruptHandler;
});

/// Pin assignments for the Seeed Xiao nRF52840 board
///
/// LoRa Radio (SX126x):
/// - NSS (Chip Select): P0_04
/// - Reset: P0_28  
/// - DIO1: P0_03
/// - Busy: P0_29
/// - SPI: TWISPI0 (SCK: P1_13, MISO: P1_14, MOSI: P1_15)
///
/// LEDs:
/// - Red: P0_26
/// - Green: P0_30
/// - Blue: P0_06
pub fn init_board(p: embassy_nrf::Peripherals) -> BoardPeripherals {
    // Initialize external high-frequency oscillator for USB
    pac::CLOCK.tasks_hfclkstart().write_value(1);
    while pac::CLOCK.events_hfclkstarted().read() != 1 {}

    // Configure LoRa radio pins
    let nss = Output::new(p.P0_04, Level::High, OutputDrive::Standard);
    let reset = Output::new(p.P0_28, Level::High, OutputDrive::Standard);
    let dio1 = Input::new(p.P0_03, Pull::Down);
    let busy = Input::new(p.P0_29, Pull::None);

    // Configure SPI for LoRa radio
    let mut spi_config = spim::Config::default();
    spi_config.frequency = spim::Frequency::M16;
    let spi_sck = p.P1_13;
    let spi_miso = p.P1_14;
    let spi_mosi = p.P1_15;
    let spim = spim::Spim::new(p.TWISPI1, Irqs, spi_sck, spi_miso, spi_mosi, spi_config);
    let spi = ExclusiveDevice::new(spim, nss, Delay);

    // Configure LEDs (active low)
    let led_red = Output::new(p.P0_26, Level::High, OutputDrive::Standard);
    let led_green = Output::new(p.P0_30, Level::High, OutputDrive::Standard);
    let led_blue = Output::new(p.P0_06, Level::High, OutputDrive::Standard);

    // Configure USB driver
    let usb_driver = usb::Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs)); // Configure RNG
    let rng = rng::Rng::new_blocking(p.RNG);
    BoardPeripherals {
        lora: LoRaPeripherals {
            spi,
            reset,
            dio1,
            busy,
        },
        leds: Some(LedPeripherals {
            red: led_red,
            green: led_green,
            blue: led_blue,
        }),
        usb_driver,
        rng,
        i2c: None,
    }
}
