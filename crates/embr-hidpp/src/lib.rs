//! HID++ over Linux `hidraw`, written from the protocol documentation and from byte-level
//! measurements of real hardware. Nothing here is copied from another implementation.
//!
//! The crate is deliberately headless and dependency-light: a `File` on `/dev/hidrawN`,
//! `poll(2)` for timeouts, and byte slices. That keeps the whole protocol layer testable
//! without a display, without an event loop and without a HID library.
//!
//! Layers, from the bottom:
//!
//! * [`report`] — HID++ frame layouts (short 7-byte, long 20-byte) and feature ids.
//! * [`transport`] — [`transport::Channel`]: open a hidraw node, send a request, read the
//!   matching reply within a deadline, close it again.
//! * [`sysfs`] — which Logitech nodes exist, what they are, and how they relate to a
//!   receiver.
//! * [`battery`] — decoding battery state, plus our own voltage-to-percentage estimate.
//! * [`receiver`] — the pairing table of a receiver (WPID, serial, name per slot).
//! * [`discover`] — the one call an application needs: [`discover::discover`].
//! * [`hotplug`] — [`hotplug::Uevents`]: the kernel saying that a HID device came or went.

pub mod battery;
pub mod decode;
pub mod discover;
pub mod hotplug;
pub mod receiver;
pub mod report;
pub mod sysfs;
pub mod transport;

pub use battery::{BatteryReading, ChargeStatus, Percent, PercentSource};
pub use decode::BatterySource;
pub use discover::{Connection, Device, DeviceKind, DiscoveryReport, discover};
pub use hotplug::Uevents;
pub use report::Feature;
pub use transport::Channel;

/// Everything that can go wrong talking to a device.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no reply after {waited:?}")]
    Timeout { waited: std::time::Duration },
    #[error("HID++ 1.0 error 0x{code:02X} ({name})")]
    Protocol1 { code: u8, name: &'static str },
    #[error("HID++ 2.0 error 0x{code:02X} ({name})")]
    Protocol2 { code: u8, name: &'static str },
    #[error("malformed reply: {0}")]
    Malformed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// HID++ 1.0 error codes (register and device-index level).
pub fn protocol1_error_name(code: u8) -> &'static str {
    match code {
        0x00 => "success",
        0x01 => "invalid sub-id/command",
        0x02 => "invalid address",
        0x03 => "invalid value",
        0x04 => "connection request failed",
        0x05 => "too many devices",
        0x06 => "sequence timeout",
        0x07 => "sequence error",
        0x08 => "unknown device",
        0x09 => "bad argument",
        0x0a => "other error",
        _ => "unrecognised",
    }
}

/// HID++ 2.0 error codes (feature/function level).
pub fn protocol2_error_name(code: u8) -> &'static str {
    match code {
        0x00 => "no error",
        0x01 => "unknown",
        0x02 => "invalid argument",
        0x03 => "out of range",
        0x04 => "hardware error",
        0x05 => "not allowed",
        0x06 => "unsupported",
        0x07 => "invalid feature index",
        0x08 => "invalid function",
        0x09 => "busy",
        0x0a => "unsupported",
        0x0b => "unsupported feature",
        0x0c => "invalid request",
        0x0d => "unsupported",
        0x0e => "unsupported",
        0x0f => "unsupported",
        _ => "unrecognised",
    }
}
