//! Template board configuration without LEDs
//!
//! This is an example template for boards that don't have user-controllable LEDs.
//! Copy this file and modify the pin assignments for your specific board.

use super::{BoardPeripherals, LoRaPeripherals};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::{adc, bind_interrupts, i2c, pac, peripherals, pio, spi, uart, usb};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Delay;
use embedded_hal_bus::spi::ExclusiveDevice;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    //ADC_IRQ_FIFO => adc::InterruptHandler;
    I2C0_IRQ => i2c::InterruptHandler<peripherals::I2C0>;
    //I2C1_IRQ => i2c::InterruptHandler<peripherals::I2C1>;
    //PIO0_IRQ_0 => pio::InterruptHandler<peripherals::PIO0>;
    //UART0_IRQ => uart::InterruptHandler<peripherals::UART0>;
    USBCTRL_IRQ => usb::InterruptHandler<peripherals::USB>;
});

/// Pin assignments for a board without user LEDs
///
/// LoRa Radio (Pico-LoRa-SX1262 hat):
/// - NSS (Chip Select): PIN_3
/// - Reset: PIN_15
/// - DIO1: PIN_20
/// - Busy: PIN_2
/// - SPI: SPI1 (SCK: PIN_10, MISO: PIN_12, MOSI: PIN_11, TX_DMA: DMA_CH0, RX_DMA: DMA_CH1)
/// - I2C: (I2C0 SCL: PIN_5, SDA: PIN_4)
///
/// This board has no user-controllable LEDs
pub fn init_board(p: embassy_rp::Peripherals) -> BoardPeripherals {
    // Configure LoRa radio pins (replace with actual pins for your board)
    let nss = Output::new(p.PIN_3, Level::High);
    let reset = Output::new(p.PIN_15, Level::High);
    let dio1 = Input::new(p.PIN_20, Pull::Down);
    let busy = Input::new(p.PIN_2, Pull::None);

    // Configure SPI for LoRa radio (replace with actual pins for your board)
    let spi_config = spi::Config::default();
    let spi_sck = p.PIN_10;
    let spi_miso = p.PIN_12;
    let spi_mosi = p.PIN_11;
    let spi_tx_dma = p.DMA_CH0;
    let spi_rx_dma = p.DMA_CH1;
    let spi_dev = spi::Spi::new(
        p.SPI1, spi_sck, spi_mosi, spi_miso, spi_tx_dma, spi_rx_dma, spi_config,
    );
    let spi = ExclusiveDevice::new(spi_dev, nss, Delay);

    // Configure USB driver
    let usb_driver = usb::Driver::new(p.USB, Irqs);

    // Configure RNG
    //TODO: investigate actual solution as this amy be wrong
    let rng = embassy_rp::clocks::RoscRng;

    // I2C bus config
    let i2c_config = i2c::Config::default();
    let i2c_scl = p.PIN_5;
    let i2c_sda = p.PIN_4;
    static I2C_BUS: StaticCell<
        Mutex<NoopRawMutex, i2c::I2c<'static, peripherals::I2C0, i2c::Async>>,
    > = StaticCell::new();
    let i2c = i2c::I2c::new_async(p.I2C0, i2c_scl, i2c_sda, Irqs, i2c_config);
    let i2c_bus = I2C_BUS.init(Mutex::new(i2c));

    BoardPeripherals {
        lora: LoRaPeripherals {
            spi,
            reset,
            dio1,
            busy,
        },
        leds: None, // This board has no LEDs
        usb_driver,
        rng,
        i2c: Some(i2c_bus),
    }
}
