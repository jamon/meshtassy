#![no_std]

/// Sensor specific code
pub mod sensors;

/// Environmental Telemetry code
pub mod environmental_telemetry;

/// Proxy struct for remote device structs
pub struct TelemetrySensor<T> {
    pub device: T,
}

/// Proxy struct for remote device errors that lack defmt support
struct RemoteError<E> {
    error: E,
}
