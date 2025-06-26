# Board Support

This directory contains board-specific configurations for the Meshtastic nRF firmware.

## Overview

The board abstraction system allows the main application code to be hardware-agnostic by:

1. **Pin Abstraction**: Board files handle all pin assignments and peripheral configuration
2. **Feature-Based Selection**: Cargo features control which board configuration is compiled
3. **Structured Peripherals**: Board-specific peripherals are organized into logical groups

## Supported Boards

### Seeed Xiao nRF52840 (`board-seeed-xiao-nrf52840`)

**Pin Assignments:**
- **LoRa Radio (SX126x):**
  - NSS (Chip Select): P0_04
  - Reset: P0_28
  - DIO1: P0_03
  - Busy: P0_29
  - SPI: TWISPI0 (SCK: P1_13, MISO: P1_14, MOSI: P1_15)

- **LEDs:**
  - Red: P0_26
  - Green: P0_30
  - Blue: P0_06

- **Other Peripherals:**
  - USB: USBD
  - RNG: RNG

## Usage

### Building for a Specific Board

The default board is `seeed-xiao-nrf52840`:
```bash
cargo build
```

To explicitly specify a board:
```bash
cargo build --features board-seeed-xiao-nrf52840
```

### Adding a New Board

1. Create a new board file in `src/boards/your_board.rs`
2. Add the board feature to `Cargo.toml`:
   ```toml
   [features]
   board-your-board = []
   ```
3. Add conditional compilation to `mod.rs`:
   ```rust
   #[cfg(feature = "board-your-board")]
   pub mod your_board;
   
   #[cfg(feature = "board-your-board")]
   pub fn init_board(p: embassy_nrf::Peripherals) -> BoardPeripherals {
       your_board::init_board(p)
   }
   ```
4. Implement the `init_board` function in your board file following the existing pattern

### Board Peripheral Structure

The board abstraction provides these peripheral groups:

```rust
pub struct BoardPeripherals {
    pub lora: LoRaPeripherals,           // LoRa radio SPI and control pins
    pub leds: Option<LedPeripherals>,    // LED outputs (optional)
    pub usb_driver: usb::Driver,         // USB driver
    pub rng: rng::Rng,                  // Random number generator
}
```

**LoRa Peripherals:**
```rust
pub struct LoRaPeripherals {
    pub spi: ExclusiveDevice<Spim, Output, Delay>,  // SPI with CS pin
    pub reset: Output,                              // Reset pin (active low)
    pub dio1: Input,                               // DIO1 interrupt pin
    pub busy: Input,                               // Busy status pin
}
```

**LED Peripherals (Optional):**
```rust
pub struct LedPeripherals {
    pub red: Output,    // Red LED
    pub green: Output,  // Green LED
    pub blue: Output,   // Blue LED
}
```

### Optional Peripherals

Some peripheral groups are optional since not all boards have them:

- **LEDs**: The `leds` field is `Option<LedPeripherals>`. If a board has no user-controllable LEDs, this will be `None`.

The main application code checks for optional peripherals:

```rust
// Handle optional LEDs
if let Some(leds) = board.leds {
    let mut led_red = leds.red;
    // ... use LEDs
    info!("LEDs initialized");
} else {
    info!("No LEDs available on this board");
}
```

### Template Files

Use these templates as starting points for new boards:

- **`seeed_xiao_nrf52840.rs`**: Board with RGB LEDs (full example)
- **`template_no_leds.rs`**: Board without user LEDs (template)

Copy the appropriate template and modify the pin assignments for your specific board.

## Design Benefits

1. **Hardware Abstraction**: Main application code doesn't need to know pin assignments
2. **Easy Porting**: Adding support for new boards only requires creating a board file
3. **Compile-Time Safety**: Incorrect board configurations are caught at compile time
4. **Feature-Based**: Use Cargo features to select boards without runtime overhead
5. **Modular**: Each board is self-contained in its own module
