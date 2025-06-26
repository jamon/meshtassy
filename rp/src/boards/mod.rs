//! Board-specific configurations and pin assignments
//!
//! This module provides board-specific abstractions that isolate
//! hardware dependencies from the main application logic.

use embassy_rp::clocks;
use embassy_rp::gpio::{Input, Output};
use embassy_rp::i2c;
use embassy_rp::peripherals;
use embassy_rp::spi;
use embassy_rp::usb;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Delay;
use embedded_hal_bus::spi::ExclusiveDevice;

// Import board-specific modules based on features
#[cfg(feature = "board-pico-rp2040")]
pub mod raspberry_pi_pico;

/// I2CBus type alias
pub type I2CBus<'dev> = i2c::I2c<'dev, peripherals::I2C0, i2c::Async>;

/// Board-specific peripheral configuration
pub struct BoardPeripherals {
    /// LoRa radio peripherals and pins
    pub lora: LoRaPeripherals,
    /// LED outputs (optional - not all boards have LEDs)
    pub leds: Option<LedPeripherals>,
    /// USB driver
    pub usb_driver: usb::Driver<'static, peripherals::USB>,
    /// Random number generator
    pub rng: clocks::RoscRng,
    /// I2C bus config
    pub i2c:
        Option<&'static mut Mutex<NoopRawMutex, i2c::I2c<'static, peripherals::I2C0, i2c::Async>>>,
}

/// LoRa radio-related peripherals
pub struct LoRaPeripherals {
    /// SPI device for communicating with the LoRa radio
    pub spi:
        ExclusiveDevice<spi::Spi<'static, peripherals::SPI1, spi::Async>, Output<'static>, Delay>,
    /// Reset pin (active low)
    pub reset: Output<'static>,
    /// DIO1 interrupt pin
    pub dio1: Input<'static>,
    /// Busy status pin
    pub busy: Input<'static>,
}

/// LED peripheral outputs
pub struct LedPeripherals {
    /// Red LED output
    pub red: Output<'static>,
    /// Green LED output  
    pub green: Output<'static>,
    /// Blue LED output
    pub blue: Output<'static>,
}

/// Initialize board-specific peripherals
///
/// This function takes the raw nRF52840 peripherals and configures them
/// according to the selected board's pin assignment and requirements.
#[cfg(feature = "board-pico-rp2040")]
pub fn init_board(p: embassy_rp::Peripherals) -> BoardPeripherals {
    raspberry_pi_pico::init_board(p)
}

// Default fallback if no board is selected
#[cfg(not(any(feature = "board-pico-rp2040",)))]
compile_error!("No board selected! Please enable a board feature like 'board-seeed-xiao-nrf52840'");
