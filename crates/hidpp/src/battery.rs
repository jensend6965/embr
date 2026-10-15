//! Battery state: what a device reports, and what we infer when it reports little.
//!
//! Two sources exist in HID++:
//!
//! * `BATTERY_STATUS {1000}` — a percentage and a charge status, straight from the device.
//! * `BATTERY_VOLTAGE {1001}` — millivolts and the same charge status.
//!
//! Plenty of Logitech devices expose only the second one, so a percentage has to be
//! inferred from voltage. That inference is *ours*: a documented monotone curve for a
//! single-cell lithium-ion cell, deliberately kept as data so a measured curve can replace
//! it. Values that come from a curve are marked [`PercentSource::Estimated`] and the UI is
//! expected to present them as approximate — the millivolts are always shown alongside, so
//! the user can see the hard number.

use std::time::SystemTime;

/// Charge state as HID++ reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeStatus {
    Discharging,
    Recharging,
    AlmostFull,
    Full,
    SlowRecharge,
    InvalidBattery,
    ThermalError,
    /// The device answered but did not say.
    Unknown,
}

impl ChargeStatus {
    /// Decode the HID++ status value.
    pub fn from_hidpp(value: u8) -> Self {
        match value & 0x0F {
            0 => Self::Discharging,
            1 => Self::Recharging,
            2 => Self::AlmostFull,
            3 => Self::Full,
            4 => Self::SlowRecharge,
            5 => Self::InvalidBattery,
            6 => Self::ThermalError,
            _ => Self::Unknown,
        }
    }

    /// Whether the device is on external power right now.
    pub fn charging(self) -> bool {
        matches!(
            self,
            Self::Recharging | Self::AlmostFull | Self::Full | Self::SlowRecharge
        )
    }

    /// A battery the device itself calls broken should not produce threshold alerts.
    pub fn faulty(self) -> bool {
        matches!(self, Self::InvalidBattery | Self::ThermalError)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Discharging => "Discharging",
            Self::Recharging => "Charging",
            Self::AlmostFull => "Almost full",
            Self::Full => "Full",
            Self::SlowRecharge => "Slow charging",
            Self::InvalidBattery => "Battery fault",
            Self::ThermalError => "Thermal fault",
            Self::Unknown => "Unknown",
        }
    }
}

/// Where a percentage came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PercentSource {
    /// The device reported it.
    Device,
    /// Inferred from millivolts with [`estimate_percent`].
    Estimated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Percent {
    pub value: u8,
    pub source: PercentSource,
}

impl Percent {
    /// `Some(88)` for a device value, `Some(88)` for an estimate — callers that need to say
    /// "about" should look at [`Percent::is_estimate`].
    pub fn is_estimate(self) -> bool {
        self.source == PercentSource::Estimated
    }
}

impl std::fmt::Display for Percent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.source {
            PercentSource::Device => write!(f, "{}%", self.value),
            PercentSource::Estimated => write!(f, "~{}%", self.value),
        }
    }
}

/// One reading of one battery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatteryReading {
    pub percent: Option<Percent>,
    pub millivolts: Option<u16>,
    pub status: ChargeStatus,
    pub captured: SystemTime,
}

impl BatteryReading {
    pub fn new(percent: Option<Percent>, millivolts: Option<u16>, status: ChargeStatus) -> Self {
        Self {
            percent,
            millivolts,
            status,
            captured: SystemTime::now(),
        }
    }

    pub fn charging(&self) -> bool {
        self.status.charging()
    }

    /// True when there is nothing useful to show: no percentage and no voltage.
    pub fn is_empty(&self) -> bool {
        self.percent.is_none() && self.millivolts.is_none()
    }

    /// Human summary, e.g. `88% (discharging, 4048 mV)`.
    pub fn summary(&self) -> String {
        match (self.percent, self.millivolts) {
            (Some(p), Some(mv)) => format!("{p} ({}, {mv} mV)", self.status.label().to_lowercase()),
            (Some(p), None) => format!("{p} ({})", self.status.label().to_lowercase()),
            (None, Some(mv)) => format!("{} mV ({})", mv, self.status.label().to_lowercase()),
            (None, None) => self.status.label().to_string(),
        }
    }
}

/// Our default voltage→percentage curve: `(millivolts, percent)` anchors, descending.
///
/// A single lithium-ion cell sits near 4.2 V full and falls off a cliff below ~3.4 V, with
/// a long flat plateau in the middle — which is why a straight line from 4.2 V to 3.0 V
/// reads far too high for most of the discharge. These anchors follow that shape. They are
/// an approximation for voltage-only devices; [`estimate_percent`] interpolates between
/// them, and the whole table can be replaced by a measured one.
pub const VOLTAGE_CURVE: &[(u16, u8)] = &[
    (4200, 100),
    (4100, 93),
    (4000, 85),
    (3950, 79),
    (3900, 72),
    (3850, 63),
    (3800, 54),
    (3750, 44),
    (3700, 34),
    (3650, 25),
    (3600, 17),
    (3550, 11),
    (3500, 6),
    (3400, 2),
    (3300, 0),
];

/// Estimate a percentage from millivolts using [`VOLTAGE_CURVE`].
pub fn estimate_percent(millivolts: u16) -> u8 {
    estimate_percent_with(millivolts, VOLTAGE_CURVE)
}

/// Estimate a percentage from millivolts with an explicit curve.
///
/// The curve must be sorted by descending millivolts; values above or below the curve's
/// range clamp to its ends, so this never fails.
pub fn estimate_percent_with(millivolts: u16, curve: &[(u16, u8)]) -> u8 {
    if curve.is_empty() {
        return 0;
    }
    let (top_mv, top_pct) = curve[0];
    if millivolts >= top_mv {
        return top_pct;
    }
    let (bottom_mv, bottom_pct) = curve[curve.len() - 1];
    if millivolts <= bottom_mv {
        return bottom_pct;
    }
    for window in curve.windows(2) {
        let (hi_mv, hi_pct) = window[0];
        let (lo_mv, lo_pct) = window[1];
        if millivolts <= hi_mv && millivolts >= lo_mv {
            let span = hi_mv - lo_mv;
            if span == 0 {
                return lo_pct;
            }
            let travelled = hi_mv - millivolts;
            let hi = u32::from(hi_pct);
            let lo = u32::from(lo_pct);
            let delta = (hi - lo) * u32::from(travelled) / u32::from(span);
            return hi.saturating_sub(delta) as u8;
        }
    }
    bottom_pct
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_decoding_matches_the_protocol() {
        assert_eq!(ChargeStatus::from_hidpp(0), ChargeStatus::Discharging);
        assert_eq!(ChargeStatus::from_hidpp(1), ChargeStatus::Recharging);
        assert_eq!(ChargeStatus::from_hidpp(3), ChargeStatus::Full);
        assert_eq!(ChargeStatus::from_hidpp(6), ChargeStatus::ThermalError);
        assert_eq!(
            ChargeStatus::from_hidpp(0xF0),
            ChargeStatus::Discharging,
            "high nibble carries version info"
        );
        assert!(ChargeStatus::Full.charging());
        assert!(!ChargeStatus::Discharging.charging());
        assert!(ChargeStatus::InvalidBattery.faulty());
    }

    #[test]
    fn the_curve_is_monotone_and_clamped() {
        let mut previous = 101;
        for (mv, pct) in VOLTAGE_CURVE {
            assert!(
                *pct <= previous,
                "curve must not rise with falling voltage at {mv} mV"
            );
            previous = *pct;
        }
        assert_eq!(estimate_percent(4200), 100);
        assert_eq!(estimate_percent(5000), 100);
        assert_eq!(estimate_percent(3300), 0);
        assert_eq!(estimate_percent(2500), 0);
    }

    #[test]
    fn the_curve_interpolates_between_anchors() {
        // exactly on an anchor
        assert_eq!(estimate_percent(3850), 63);
        // half way from 3850 (63%) down to 3800 (54%)
        let midpoint = estimate_percent(3825);
        assert!(
            (54..=63).contains(&midpoint),
            "expected an interpolated value, got {midpoint}"
        );
        // monotone over the whole plausible range: walking *down* in voltage, the
        // percentage must never rise
        let mut last = 101;
        for mv in (3200..=4300).rev().step_by(10) {
            let pct = estimate_percent(mv);
            assert!(
                pct <= last,
                "percentage rose from {last} to {pct} while voltage fell to {mv} mV"
            );
            last = pct;
        }
    }

    #[test]
    fn an_empty_curve_does_not_panic() {
        assert_eq!(estimate_percent_with(4000, &[]), 0);
    }

    #[test]
    fn estimates_are_marked_as_estimates() {
        let reading = BatteryReading::new(
            Some(Percent {
                value: 88,
                source: PercentSource::Estimated,
            }),
            Some(4048),
            ChargeStatus::Discharging,
        );
        assert!(reading.percent.unwrap().is_estimate());
        assert_eq!(reading.summary(), "~88% (discharging, 4048 mV)");
        assert!(!reading.is_empty());
    }
}
