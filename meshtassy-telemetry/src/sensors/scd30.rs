use defmt::*;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::Delay;
use embedded_hal::i2c::ErrorType;
use embedded_hal_async::i2c::I2c;
use femtopb::UnknownFields;
use libscd::asynchronous::scd30::Scd30;
use meshtastic_protobufs::meshtastic::EnvironmentMetrics;

use crate::{TelemetrySensor, environmental_telemetry::EnvironmentData};

/// Alias SCD30 typedef for shorter name
#[allow(dead_code)]
pub type SCD30<'dev, BUS> = Scd30<I2cDevice<'dev, NoopRawMutex, BUS>, Delay>;

/// Implement TelemetrySensor on the BME
impl<'dev, BUS: I2c + ErrorType + 'static> TelemetrySensor<SCD30<'dev, BUS>> {
    pub fn new(bus: I2cDevice<'dev, NoopRawMutex, BUS>) -> Self {
        Self {
            device: Scd30::new(bus, Delay),
        }
    }
}

/// Implement EnvironmentData for SCD30
impl<BUS: I2c + ErrorType> EnvironmentData for TelemetrySensor<SCD30<'static, BUS>>
where
    <BUS as ErrorType>::Error: defmt::Format,
{
    async fn setup(&mut self) {
        // not much is required initially here. perhaps eventually this runs the calibration routine
    }
    async fn get_metrics(&mut self) -> Option<EnvironmentMetrics<'_>> {
        if self.device.data_ready().await.is_ok_and(|b| b == true) {
            match self.device.read_measurement().await {
                Ok(data) => {
                    info!(
                        "SCD30 get_metrics()\n\t\t Temperature: {:?}\n\t\t Humidity: {:?}",
                        data.temperature, data.humidity
                    );
                    Some(EnvironmentMetrics {
                        temperature: Some(data.temperature),
                        relative_humidity: Some(data.humidity),
                        barometric_pressure: None,
                        gas_resistance: None,
                        voltage: None,
                        current: None,
                        iaq: None,
                        distance: None,
                        lux: None,
                        white_lux: None,
                        ir_lux: None,
                        uv_lux: None,
                        wind_direction: None,
                        wind_speed: None,
                        weight: None,
                        wind_gust: None,
                        wind_lull: None,
                        radiation: None,
                        rainfall_1h: None,
                        rainfall_24h: None,
                        soil_moisture: None,
                        soil_temperature: None,
                        unknown_fields: UnknownFields::default(),
                    })
                }
                Err(e) => {
                    error!("Could not get measurements from SCD30: {:?}", e);
                    None
                }
            }
        } else {
            None
        }
    }
}
