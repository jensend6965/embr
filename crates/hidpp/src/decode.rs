//! Turning HID++ reply payloads into [`BatteryReading`]s.
//!
//! Three features report battery state, and devices implement different subsets:
//!
//! | Feature | Function 0 payload | Gives |
//! | --- | --- | --- |
//! | `BATTERY_STATUS {1000}` | `level% next% status` | a real percentage |
//! | `BATTERY_VOLTAGE {1001}` | `millivolts:u16 flags` | voltage (+ charge state in the flags) |
//! | `UNIFIED_BATTERY {1004}` | `charge% level-nibble status unused` | percentage with a coarse level fallback |
//!
//! A device that reports only voltage is the common case (both devices on the development
//! machine are), so the percentage then comes from our own curve and is marked estimated.
//!
//! `BATTERY_VOLTAGE` flags, as the protocol defines them:
//!
//! ```text
//! bit 7  on external power (recharging)
//! bit 5  charge level critical
//! bit 4  slow recharge
//! bit 3  fast charge
//! bit 2  charging complete
//! bits 1..0  charge status detail (only meaningful while bit 7 is set)
//! ```

use crate::battery::{BatteryReading, ChargeStatus, Percent, PercentSource, estimate_percent};

/// Which feature a reading came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatterySource {
    Status,
    Voltage,
    Unified,
}

impl BatterySource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Status => "BATTERY_STATUS {1000}",
            Self::Voltage => "BATTERY_VOLTAGE {1001}",
            Self::Unified => "UNIFIED_BATTERY {1004}",
        }
    }
}

/// Charge state from a `BATTERY_VOLTAGE` flags byte.
///
/// The fine-grained sub-states in bits 1..0 are not decoded: a device on external power is
/// reported as charging (or slow-charging), which is what the user needs to see.
pub fn charge_status_from_voltage_flags(flags: u8) -> ChargeStatus {
    if flags & 0x10 != 0 {
        ChargeStatus::SlowRecharge
    } else if flags & 0x80 != 0 {
        if flags & 0x04 != 0 {
            ChargeStatus::AlmostFull
        } else {
            ChargeStatus::Recharging
        }
    } else {
        ChargeStatus::Discharging
    }
}

/// Whether the flags say nothing about charging, i.e. the battery looks unreadable.
pub fn voltage_flags_are_faulty(flags: u8) -> bool {
    flags & 0x02 != 0 && flags & 0x80 == 0 && flags & 0x10 == 0
}

/// Decode a `BATTERY_STATUS {1000}` payload.
pub fn battery_status(payload: &[u8]) -> Option<BatteryReading> {
    if payload.len() < 3 {
        return None;
    }
    let level = payload[0];
    let status = ChargeStatus::from_hidpp(payload[2]);
    let percent = match level {
        // 0 means "the device has no percentage to give"
        0 => None,
        value => Some(Percent {
            value: value.min(100),
            source: PercentSource::Device,
        }),
    };
    Some(BatteryReading::new(percent, None, status))
}

/// Decode a `BATTERY_VOLTAGE {1001}` payload.
pub fn battery_voltage(payload: &[u8]) -> Option<BatteryReading> {
    if payload.len() < 3 {
        return None;
    }
    let millivolts = u16::from_be_bytes([payload[0], payload[1]]);
    let flags = payload[2];
    if millivolts == 0 {
        // No voltage at all: report the charge state, but invent nothing.
        return Some(BatteryReading::new(
            None,
            None,
            charge_status_from_voltage_flags(flags),
        ));
    }
    let status = charge_status_from_voltage_flags(flags);
    let percent = Percent {
        value: estimate_percent(millivolts),
        source: PercentSource::Estimated,
    };
    Some(BatteryReading::new(Some(percent), Some(millivolts), status))
}

/// Decode a `UNIFIED_BATTERY {1004}` payload.
pub fn unified_battery(payload: &[u8]) -> Option<BatteryReading> {
    if payload.len() < 3 {
        return None;
    }
    let charge = payload[0];
    let level_nibble = payload[1];
    let status = ChargeStatus::from_hidpp(payload[2]);
    let percent = if charge == 0 {
        coarse_level_percent(level_nibble).map(|value| Percent {
            value,
            source: PercentSource::Estimated,
        })
    } else {
        Some(Percent {
            value: charge.min(100),
            source: PercentSource::Device,
        })
    };
    Some(BatteryReading::new(percent, None, status))
}

/// The `UNIFIED_BATTERY` level nibble, as the protocol describes it.
pub fn coarse_level_percent(level: u8) -> Option<u8> {
    match level {
        8 => Some(100),
        4 => Some(50),
        2 => Some(20),
        1 => Some(5),
        0 => Some(0),
        _ => None,
    }
}

/// Fold two readings of the same battery into the richest one.
///
/// Order of preference: a device-reported percentage beats an estimate, a real status byte
/// beats one inferred from voltage flags, and voltage is kept whenever it is known.
pub fn merge(primary: BatteryReading, secondary: BatteryReading) -> BatteryReading {
    let percent = match (primary.percent, secondary.percent) {
        (Some(a), Some(b)) => match (a.source, b.source) {
            (PercentSource::Device, _) => Some(a),
            (_, PercentSource::Device) => Some(b),
            // Two estimates: the one that came with a real voltage is finer than one
            // derived from a coarse level nibble.
            (PercentSource::Estimated, PercentSource::Estimated) => {
                if primary.millivolts.is_some() || secondary.millivolts.is_none() {
                    Some(a)
                } else {
                    Some(b)
                }
            }
        },
        (Some(a), None) => Some(a),
        (None, b) => b,
    };
    let status = if primary.status == ChargeStatus::Unknown {
        secondary.status
    } else {
        primary.status
    };
    BatteryReading {
        percent,
        millivolts: primary.millivolts.or(secondary.millivolts),
        status,
        captured: primary.captured,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battery::estimate_percent;

    #[test]
    fn voltage_reply_from_real_hardware_decodes() {
        // verbatim: G703 LIGHTSPEED, 0x1001 at index 6, payload "0F D2 00" = 4050 mV
        let reading = battery_voltage(&[0x0F, 0xD2, 0x00]).unwrap();
        assert_eq!(reading.millivolts, Some(4050));
        assert_eq!(reading.status, ChargeStatus::Discharging);
        let percent = reading.percent.unwrap();
        assert!(
            percent.is_estimate(),
            "voltage-only devices must be marked as estimated"
        );
        assert_eq!(percent.value, estimate_percent(4050));
        assert!(
            reading.summary().starts_with('~'),
            "summary must show the estimate marker"
        );
    }

    #[test]
    fn voltage_flags_say_what_the_device_is_doing() {
        assert_eq!(
            charge_status_from_voltage_flags(0x00),
            ChargeStatus::Discharging
        );
        assert_eq!(
            charge_status_from_voltage_flags(0x0F),
            ChargeStatus::Discharging,
            "0x0F is what a discharging G703 sends"
        );
        assert_eq!(
            charge_status_from_voltage_flags(0x80),
            ChargeStatus::Recharging
        );
        assert_eq!(
            charge_status_from_voltage_flags(0x84),
            ChargeStatus::AlmostFull
        );
        assert_eq!(
            charge_status_from_voltage_flags(0x90),
            ChargeStatus::SlowRecharge
        );
    }

    #[test]
    fn status_reply_carries_the_device_percentage() {
        // level 88, next 20, status 0 = discharging
        let reading = battery_status(&[88, 20, 0]).unwrap();
        let percent = reading.percent.unwrap();
        assert_eq!(percent.value, 88);
        assert_eq!(percent.source, PercentSource::Device);
        assert!(!percent.is_estimate());
        // level 0 means "no percentage", not "empty"
        assert_eq!(battery_status(&[0, 20, 0]).unwrap().percent, None);
        assert!(
            battery_status(&[1, 2]).is_none(),
            "short payloads are rejected"
        );
    }

    #[test]
    fn unified_battery_falls_back_to_its_coarse_level() {
        let exact = unified_battery(&[73, 4, 0, 0]).unwrap();
        assert_eq!(exact.percent.unwrap().value, 73);
        assert_eq!(exact.percent.unwrap().source, PercentSource::Device);
        let coarse = unified_battery(&[0, 2, 0, 0]).unwrap();
        assert_eq!(coarse.percent.unwrap().value, 20);
        assert!(coarse.percent.unwrap().is_estimate());
    }

    #[test]
    fn merging_prefers_the_device_percentage_and_keeps_voltage() {
        let estimated = battery_voltage(&[0x0F, 0xD2, 0x00]).unwrap();
        let device = battery_status(&[88, 20, 0]).unwrap();
        let merged = merge(device, estimated);
        assert_eq!(merged.percent.unwrap().value, 88);
        assert_eq!(merged.percent.unwrap().source, PercentSource::Device);
        assert_eq!(merged.millivolts, Some(4050));
        assert_eq!(merged.status, ChargeStatus::Discharging);
        // and the other way round, with an unknown primary status
        let merged = merge(
            battery_status(&[0, 0, 0xFF]).unwrap(),
            battery_voltage(&[0x0F, 0xD2, 0x00]).unwrap(),
        );
        assert_eq!(merged.percent.unwrap().source, PercentSource::Estimated);
    }
}
