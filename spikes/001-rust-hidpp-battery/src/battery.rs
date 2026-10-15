//! Battery decode + the percentage estimation Solaar uses for voltage-only devices.
//!
//! Layouts verified against Solaar 1.1.20:
//!   hidpp20.decipher_battery_status : "!BBB"  level%, next level, status enum
//!   hidpp20.decipher_battery_voltage: ">HB"   millivolts BE, flags byte
//!   hidpp10.parse_battery_status    : register 0x07 / 0x0D

use crate::hidpp::hex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Discharging,
    Recharging,
    AlmostFull,
    Full,
    SlowRecharge,
    InvalidBattery,
    ThermalError,
}

impl Status {
    pub fn from_enum(b: u8) -> Option<Status> {
        match b & 0x0F {
            0 => Some(Status::Discharging),
            1 => Some(Status::Recharging),
            2 => Some(Status::AlmostFull),
            3 => Some(Status::Full),
            4 => Some(Status::SlowRecharge),
            5 => Some(Status::InvalidBattery),
            6 => Some(Status::ThermalError),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Discharging => "DISCHARGING",
            Status::Recharging => "RECHARGING",
            Status::AlmostFull => "ALMOST_FULL",
            Status::Full => "FULL",
            Status::SlowRecharge => "SLOW_RECHARGE",
            Status::InvalidBattery => "INVALID_BATTERY",
            Status::ThermalError => "THERMAL_ERROR",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BatteryInfo {
    /// which HID++ call produced this, e.g. "BATTERY_VOLTAGE {1001} @idx 6"
    pub source: String,
    pub percent: Option<u8>,
    pub next_level: Option<u8>,
    pub millivolts: Option<u16>,
    pub status: Option<Status>,
    /// BATTERY_STATUS byte0 bit7: level is an approximation
    pub level_approx: Option<bool>,
    /// raw flag/level bytes, for the record
    pub flags_hex: String,
}

/// Solaar's estimate_battery_level_percentage: piecewise-linear mV -> % table.
pub fn estimate_percent(mv: u16) -> Option<u8> {
    const TABLE: [(u16, u8); 13] = [
        (4186, 100),
        (4067, 90),
        (3989, 80),
        (3922, 70),
        (3859, 60),
        (3811, 50),
        (3778, 40),
        (3751, 30),
        (3717, 20),
        (3671, 10),
        (3646, 5),
        (3579, 2),
        (3500, 0),
    ];
    if mv == 0 {
        return None;
    }
    if mv >= TABLE[0].0 {
        return Some(TABLE[0].1);
    }
    if mv <= TABLE[TABLE.len() - 1].0 {
        return Some(TABLE[TABLE.len() - 1].1);
    }
    for w in TABLE.windows(2) {
        let (v_high, p_high) = w[0];
        let (v_low, p_low) = w[1];
        if mv >= v_low && mv <= v_high {
            let pct = p_low as f64
                + (p_high as f64 - p_low as f64) * (mv as f64 - v_low as f64)
                    / (v_high as f64 - v_low as f64);
            return Some(pct.round() as u8);
        }
    }
    Some(0)
}

/// BATTERY_STATUS (0x1000) function 0: level, next level, status byte.
pub fn decode_battery_status(reply: &[u8], source: &str) -> Option<BatteryInfo> {
    if reply.len() < 3 {
        return None;
    }
    let level = reply[0];
    let next = reply[1];
    let status_byte = reply[2];
    Some(BatteryInfo {
        source: source.to_string(),
        percent: if level == 0 { None } else { Some(level) },
        next_level: Some(next),
        millivolts: None,
        status: Status::from_enum(status_byte),
        level_approx: Some(status_byte & 0x80 != 0),
        flags_hex: hex(&reply[..3]),
    })
}

/// BATTERY_VOLTAGE (0x1001) function 0: u16 big-endian mV, flags byte.
/// Flag semantics ported from Solaar's decipher_battery_voltage.
pub fn decode_battery_voltage(reply: &[u8], source: &str) -> Option<BatteryInfo> {
    if reply.len() < 3 {
        return None;
    }
    let mv = u16::from_be_bytes([reply[0], reply[1]]);
    let flags = reply[2];

    let mut status = Status::Discharging;
    let charge_sts = flags & 0x03;
    if flags & (1 << 7) != 0 {
        status = Status::Recharging;
        if charge_sts == 1 {
            status = Status::Full;
        }
    }
    if flags & (1 << 3) != 0 {
        status = Status::Recharging; // fast charge
    } else if flags & (1 << 4) != 0 {
        status = Status::SlowRecharge;
    }

    Some(BatteryInfo {
        source: source.to_string(),
        percent: estimate_percent(mv),
        next_level: None,
        millivolts: Some(mv),
        status: Some(status),
        level_approx: Some(flags & (1 << 5) != 0), // bit5: level is critical/approx
        flags_hex: hex(&reply[..3]),
    })
}

/// HID++ 1.0 register 0x07 (BATTERY_STATUS): level byte, charging byte.
/// We surface the charging byte raw plus Solaar's mapping.
pub fn decode_hidpp10_register(reply: &[u8], register: u8, source: &str) -> Option<BatteryInfo> {
    if register == 0x0D {
        if reply.len() < 3 {
            return None;
        }
        let charge = reply[0];
        let status_byte = reply[2] & 0xF0;
        let status = match status_byte {
            0x30 => Some(Status::Discharging),
            0x50 => Some(Status::Recharging),
            0x90 => Some(Status::Full),
            _ => None,
        };
        return Some(BatteryInfo {
            source: source.to_string(),
            percent: Some(charge),
            next_level: None,
            millivolts: None,
            status,
            level_approx: None,
            flags_hex: hex(&reply[..3.min(reply.len())]),
        });
    }
    if reply.len() < 2 {
        return None;
    }
    let status_byte = reply[0];
    let charging_byte = reply[1];
    let status = if charging_byte == 0x00 {
        Some(Status::Discharging)
    } else if charging_byte & 0x21 == 0x21 {
        Some(Status::Recharging)
    } else if charging_byte & 0x22 == 0x22 {
        Some(Status::Full)
    } else {
        None
    };
    // register 0x07 reports coarse level codes, not a percentage
    let approx = match status_byte & 0x0F {
        0x00 | 0x07 if status_byte != 0 => Some(100u8),
        0x05 => Some(90),
        0x03 => Some(50),
        0x01 => Some(20),
        _ => None,
    };
    Some(BatteryInfo {
        source: source.to_string(),
        percent: approx,
        next_level: None,
        millivolts: None,
        status,
        level_approx: None,
        flags_hex: hex(&reply[..2]),
    })
}

/// UNIFIED_BATTERY (0x1004) function 0x10: percent, coarse level code, status, reserved.
/// Solaar: percent falls back to the coarse code when the percentage byte is 0.
pub fn decode_unified_battery(reply: &[u8], source: &str) -> Option<BatteryInfo> {
    if reply.len() < 4 {
        return None;
    }
    let discharge = reply[0];
    let level = reply[1];
    let status_byte = reply[2];
    let coarse = match level {
        8 => Some(100u8),
        4 => Some(90),
        2 => Some(20),
        1 => Some(5),
        _ => None,
    };
    Some(BatteryInfo {
        source: source.to_string(),
        percent: if discharge != 0 { Some(discharge) } else { coarse },
        next_level: Some(level),
        millivolts: None,
        status: Status::from_enum(status_byte),
        level_approx: None,
        flags_hex: hex(&reply[..4]),
    })
}

/// ADC_MEASUREMENT (0x1F20) function 0: u16 BE millivolts + flags.
/// Solaar only treats bit0 as "measurement valid" and bit1 as "charging".
pub fn decode_adc_measurement(reply: &[u8], source: &str) -> Option<BatteryInfo> {
    if reply.len() < 3 {
        return None;
    }
    let mv = u16::from_be_bytes([reply[0], reply[1]]);
    let flags = reply[2];
    let status = if flags & 0x01 != 0 {
        Some(if flags & 0x02 != 0 { Status::Recharging } else { Status::Discharging })
    } else {
        None
    };
    Some(BatteryInfo {
        source: source.to_string(),
        percent: estimate_percent(mv),
        next_level: None,
        millivolts: Some(mv),
        status,
        level_approx: Some(flags & 0x01 == 0),
        flags_hex: hex(&reply[..3]),
    })
}
