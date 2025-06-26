//! Template board configuration without LEDs
//! 
//! This is an example template for boards that don't have user-controllable LEDs.
//! Copy this file and modify the pin assignments for your specific board.

use super::{BoardPeripherals, LoRaPeripherals};
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

/// Pin assignments for a board without user LEDs
/// 
/// LoRa Radio (SX126x):
/// - NSS (Chip Select): P0_XX (replace with actual pin)
/// - Reset: P0_XX (replace with actual pin)
/// - DIO1: P0_XX (replace with actual pin) 
/// - Busy: P0_XX (replace with actual pin)
/// - SPI: TWISPI0 (SCK: P1_XX, MISO: P1_XX, MOSI: P1_XX) (replace with actual pins)
///
/// This board has no user-controllable LEDs
pub fn init_board(p: embassy_nrf::Peripherals) -> BoardPeripherals {
    // Initialize external high-frequency oscillator for USB
    pac::CLOCK.tasks_hfclkstart().write_value(1);
    while pac::CLOCK.events_hfclkstarted().read() != 1 {}

    // Configure LoRa radio pins (replace with actual pins for your board)
    let nss = Output::new(p.P0_04, Level::High, OutputDrive::Standard);    // Replace P0_04
    let reset = Output::new(p.P0_28, Level::High, OutputDrive::Standard);  // Replace P0_28
    let dio1 = Input::new(p.P0_03, Pull::Down);                           // Replace P0_03
    let busy = Input::new(p.P0_29, Pull::None);                           // Replace P0_29

    // Configure SPI for LoRa radio (replace with actual pins for your board)
    let mut spi_config = spim::Config::default();
    spi_config.frequency = spim::Frequency::M16;
    let spi_sck = p.P1_13;   // Replace P1_13
    let spi_miso = p.P1_14;  // Replace P1_14
    let spi_mosi = p.P1_15;  // Replace P1_15
    let spim = spim::Spim::new(p.TWISPI0, Irqs, spi_sck, spi_miso, spi_mosi, spi_config);
    let spi = ExclusiveDevice::new(spim, nss, Delay);

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
        leds: None,  // This board has no LEDs
        usb_driver,
        rng,
    }
}
