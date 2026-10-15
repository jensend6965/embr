//! The view model: everything the UI needs, computed without a widget in sight.
//!
//! Widget code should format nothing and decide nothing. It takes a [`Snapshot`] and draws
//! it, which is what makes the interesting parts testable without a display.

use std::time::SystemTime;

use hidpp::discover::{Device, DiscoveryReport};

use crate::config::Config;

/// One device, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceView {
    /// Stable identity, used for snoozing and for keeping the selection across refreshes.
    pub key: String,
    pub name: String,
    pub kind: String,
    pub connection: String,
    /// 0–100, when it is known.
    pub percent: Option<u8>,
    /// The percentage as the user should see it (`88%` or `~88%`).
    pub percent_text: String,
    /// True when the percentage was inferred from voltage rather than reported.
    pub estimated: bool,
    pub millivolts: Option<u16>,
    pub status: String,
    pub charging: bool,
    /// The device itself reports a broken or overheated battery.
    pub faulty: bool,
    /// Why there is no reading.
    pub error: Option<String>,
    /// When the reading was taken.
    pub captured: Option<SystemTime>,
}

impl DeviceView {
    pub fn from_device(device: &Device, config: &Config) -> Self {
        let reading = device.battery.as_ref();
        let percent = reading.and_then(|b| b.percent);
        let estimated = percent.is_some_and(|p| p.is_estimate());
        let mark_estimates = config.ui.mark_estimates;
        Self {
            key: device.key.clone(),
            name: device.name.clone(),
            kind: device.kind.label().to_string(),
            connection: device.connection.label(),
            percent: percent.map(|p| p.value),
            percent_text: match percent {
                Some(p) if estimated && mark_estimates => format!("~{}%", p.value),
                Some(p) => format!("{}%", p.value),
                None => "—".to_string(),
            },
            estimated: estimated && mark_estimates,
            millivolts: reading.and_then(|b| b.millivolts),
            status: device.status_label().to_string(),
            charging: reading.is_some_and(|b| b.charging()),
            faulty: reading.is_some_and(|b| b.status.faulty()),
            error: device.error.clone(),
            captured: reading.map(|b| b.captured),
        }
    }

    /// Whether this device needs attention: low, or reporting a faulty battery.
    pub fn needs_attention(&self) -> bool {
        self.faulty || self.percent.is_some_and(|p| p <= 20)
    }
}

/// Everything one refresh produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub devices: Vec<DeviceView>,
    pub taken: Option<SystemTime>,
    /// Problems that are not about a single device (a receiver that did not answer, a
    /// device the kernel does not expose).
    pub warnings: Vec<String>,
}

impl Snapshot {
    /// Build a snapshot from a discovery report, worst battery first.
    pub fn from_report(report: &DiscoveryReport, config: &Config) -> Self {
        let mut devices: Vec<DeviceView> = report
            .devices
            .iter()
            .map(|d| DeviceView::from_device(d, config))
            .collect();
        sort_worst_first(&mut devices);
        Self {
            devices,
            taken: Some(SystemTime::now()),
            warnings: report.warnings.clone(),
        }
    }

    /// The device with the least charge, ignoring devices we could not read.
    pub fn lowest(&self) -> Option<&DeviceView> {
        self.devices
            .iter()
            .filter(|d| d.percent.is_some())
            .min_by_key(|d| d.percent)
    }

    /// A one-line summary for a tooltip or a window header.
    pub fn headline(&self) -> String {
        let total = self.devices.len();
        if total == 0 {
            return "No Logitech devices found".to_string();
        }
        match self.lowest() {
            Some(lowest) => format!(
                "{total} device{} · lowest {} ({})",
                if total == 1 { "" } else { "s" },
                lowest.percent_text,
                lowest.name
            ),
            None => format!(
                "{total} device{} · no battery readings",
                if total == 1 { "" } else { "s" }
            ),
        }
    }

    /// The tooltip for a tray icon: every device with a level, one per line.
    pub fn tray_tooltip(&self) -> String {
        if self.devices.is_empty() {
            return "No Logitech devices found".to_string();
        }
        let mut lines = vec!["Logitech devices".to_string()];
        for device in &self.devices {
            // A device with no reading gets its status alone: "G733 — Unavailable", never
            // "G733 — — (Unavailable)".
            let level = if device.percent.is_some() {
                format!("{} ({})", device.percent_text, device.status)
            } else {
                device.status.clone()
            };
            lines.push(format!("{} — {}", device.name, level));
        }
        lines.join("\n")
    }
}

/// Sort worst battery first; devices without a reading go last, by name.
pub fn sort_worst_first(devices: &mut [DeviceView]) {
    devices.sort_by(|a, b| match (a.percent, b.percent) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.name.cmp(&b.name)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.name.cmp(&b.name),
    });
}

/// `12s ago`, `4 min ago`, `2 h ago` — for the "updated" field of the details panel.
pub fn human_age(captured: SystemTime, now: SystemTime) -> String {
    let seconds = now
        .duration_since(captured)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match seconds {
        0..=1 => "just now".to_string(),
        2..=59 => format!("{seconds}s ago"),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86399 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hidpp::battery::{BatteryReading, ChargeStatus, Percent, PercentSource};
    use hidpp::discover::{Connection, DeviceKind};
    use std::time::Duration;

    fn device(name: &str, percent: Option<(u8, PercentSource)>) -> Device {
        let reading = percent.map(|(value, source)| {
            BatteryReading::new(
                Some(Percent { value, source }),
                Some(3700),
                ChargeStatus::Discharging,
            )
        });
        Device {
            key: name.to_lowercase(),
            name: name.to_string(),
            kind: DeviceKind::Mouse,
            connection: Connection::Usb,
            hidraw: None,
            serial: None,
            wpid: None,
            battery: reading,
            error: None,
            responded: true,
        }
    }

    fn config() -> Config {
        Config::default()
    }

    #[test]
    fn estimates_are_marked_in_the_text_the_ui_shows() {
        let reported =
            DeviceView::from_device(&device("A", Some((88, PercentSource::Device))), &config());
        assert_eq!(reported.percent_text, "88%");
        assert!(!reported.estimated);

        let inferred = DeviceView::from_device(
            &device("B", Some((88, PercentSource::Estimated))),
            &config(),
        );
        assert_eq!(inferred.percent_text, "~88%");
        assert!(inferred.estimated);

        // …unless the user turned the marking off
        let mut quiet = config();
        quiet.ui.mark_estimates = false;
        let inferred =
            DeviceView::from_device(&device("B", Some((88, PercentSource::Estimated))), &quiet);
        assert_eq!(inferred.percent_text, "88%");
        assert!(!inferred.estimated);
    }

    #[test]
    fn devices_without_a_reading_say_so() {
        let view = DeviceView::from_device(&device("C", None), &config());
        assert_eq!(view.percent_text, "—");
        assert_eq!(view.percent, None);
        assert!(!view.needs_attention());
    }

    #[test]
    fn the_worst_battery_comes_first_and_unreadable_devices_last() {
        let mut devices = vec![
            DeviceView::from_device(
                &device("Full", Some((90, PercentSource::Device))),
                &config(),
            ),
            DeviceView::from_device(&device("Unknown", None), &config()),
            DeviceView::from_device(&device("Low", Some((8, PercentSource::Device))), &config()),
        ];
        sort_worst_first(&mut devices);
        let order: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(order, vec!["Low", "Full", "Unknown"]);
    }

    #[test]
    fn the_headline_and_tooltip_are_useful_at_a_glance() {
        let report = DiscoveryReport {
            devices: vec![
                device("G703 LIGHTSPEED", Some((88, PercentSource::Estimated))),
                device("G915 TKL", None),
            ],
            warnings: vec![],
            nodes_seen: 9,
        };
        let snapshot = Snapshot::from_report(&report, &config());
        assert_eq!(
            snapshot.headline(),
            "2 devices · lowest ~88% (G703 LIGHTSPEED)"
        );
        let tooltip = snapshot.tray_tooltip();
        assert!(tooltip.starts_with("Logitech devices"), "{tooltip}");
        assert!(
            tooltip.contains("G703 LIGHTSPEED — ~88% (Discharging)"),
            "{tooltip}"
        );
        // a device without a reading must not produce a doubled "— —", and it says why: this
        // one answered us but had no battery to report
        assert!(tooltip.contains("G915 TKL — Connected"), "{tooltip}");
        assert!(!tooltip.contains("— —"), "{tooltip}");
    }

    #[test]
    fn an_empty_snapshot_says_so_instead_of_showing_zero_devices() {
        let snapshot = Snapshot::default();
        assert_eq!(snapshot.headline(), "No Logitech devices found");
        assert_eq!(snapshot.tray_tooltip(), "No Logitech devices found");
        assert!(snapshot.lowest().is_none());
    }

    #[test]
    fn ages_read_like_a_human_wrote_them() {
        let now = SystemTime::now();
        assert_eq!(human_age(now, now), "just now");
        assert_eq!(human_age(now - Duration::from_secs(30), now), "30s ago");
        assert_eq!(human_age(now - Duration::from_secs(600), now), "10 min ago");
        assert_eq!(human_age(now - Duration::from_secs(7200), now), "2 h ago");
        assert_eq!(
            human_age(now - Duration::from_secs(3 * 86400), now),
            "3 d ago"
        );
        // a clock that went backwards must not panic or print nonsense
        assert_eq!(human_age(now + Duration::from_secs(600), now), "just now");
    }

    #[test]
    fn a_low_or_faulty_battery_wants_attention() {
        let low =
            DeviceView::from_device(&device("Low", Some((15, PercentSource::Device))), &config());
        assert!(low.needs_attention());
        let fine = DeviceView::from_device(
            &device("Fine", Some((80, PercentSource::Device))),
            &config(),
        );
        assert!(!fine.needs_attention());
    }
}
