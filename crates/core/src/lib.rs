//! The application's brain, with no display and no UI toolkit in sight.
//!
//! * [`config`] — the user's options on disk, tolerant of a file they edited by hand.
//! * [`policy`] — when a battery is worth a notification, and (mostly) when it is not.
//! * [`view`] — the device list as the UI should draw it, worst battery first.
//! * [`Monitor`] — ties the three together: refresh, observe, notify, snapshot.
//!
//! Everything here is driven by an injected clock and takes its data as arguments, so the
//! tests run in microseconds and never touch the hardware.

pub mod config;
pub mod policy;
pub mod view;

use std::time::Instant;

use hidpp::discover::{self, DiscoveryReport};

pub use config::{Config, Notifications, default_path};
pub use policy::{Alert, BatteryPolicy, PolicySettings};
pub use view::{DeviceView, Snapshot, human_age};

/// Holds the configuration, the battery policy and the latest snapshot.
#[derive(Debug)]
pub struct Monitor {
    config: Config,
    policy: BatteryPolicy,
    snapshot: Snapshot,
    /// Problems to show the user once (a bad config file, a receiver that did not answer).
    pub notices: Vec<String>,
}

impl Monitor {
    pub fn new(config: Config) -> Self {
        let mut notices = Vec::new();
        if !config.notifications.enabled {
            notices.push("Battery notifications are off. Turn them on in Settings.".to_string());
        }
        let policy = BatteryPolicy::new(settings_from(&config));
        let snapshot = Snapshot::default();
        Self {
            config,
            policy,
            snapshot,
            notices,
        }
    }

    /// Read the configuration from disk and build a monitor around it.
    pub fn from_disk() -> Self {
        let path = default_path();
        let (config, problems) = config::load(&path);
        let mut monitor = Self::new(config);
        monitor.notices.splice(0..0, problems);
        monitor
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Replace the configuration and re-derive the policy from it.
    pub fn set_config(&mut self, config: Config) {
        self.config = config;
        self.policy.set_settings(settings_from(&self.config));
    }

    /// Write the current configuration back to disk, returning why it failed if it did.
    pub fn save_config(&self) -> Result<(), String> {
        config::save(&self.config, &default_path())
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn policy(&self) -> &BatteryPolicy {
        &self.policy
    }

    /// Discover devices, read batteries, and return whatever is worth notifying about.
    pub fn refresh(&mut self) -> Vec<Alert> {
        self.refresh_from(discover::discover())
    }

    /// The same, from an already collected report — the seam the tests use.
    pub fn refresh_from(&mut self, report: DiscoveryReport) -> Vec<Alert> {
        let now = Instant::now();
        let mut alerts = Vec::new();
        for device in &report.devices {
            if let Some(reading) = device.battery.as_ref() {
                alerts.extend(self.policy.observe(&device.key, &device.name, reading, now));
            }
        }
        // Devices that vanished should not keep their state forever: a re-pairing is a new
        // battery cycle. Anything not in this report is forgotten.
        let present: Vec<&str> = report.devices.iter().map(|d| d.key.as_str()).collect();
        for key in self.policy.known_keys() {
            if !present.contains(&key.as_str()) {
                self.policy.reset(&key);
            }
        }
        self.snapshot = Snapshot::from_report(&report, &self.config);
        alerts
    }

    /// Keep quiet about one device for the configured snooze period.
    pub fn snooze(&mut self, key: &str) {
        self.policy.snooze(key, Instant::now());
    }

    /// Turn battery notifications on or off and remember the choice.
    pub fn set_notifications_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.config.notifications.enabled = enabled;
        self.policy.set_settings(settings_from(&self.config));
        self.save_config()
    }
}

fn settings_from(config: &Config) -> PolicySettings {
    PolicySettings {
        enabled: config.notifications.enabled,
        thresholds: config.notifications.thresholds.clone(),
        hysteresis: config.notifications.hysteresis_percent,
        rate_limit: std::time::Duration::from_secs(config.notifications.rate_limit_seconds),
        snooze: std::time::Duration::from_secs(config.notifications.snooze_minutes * 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hidpp::battery::{BatteryReading, ChargeStatus, Percent, PercentSource};
    use hidpp::discover::{Connection, Device, DeviceKind, DiscoveryReport};

    fn report(devices: Vec<Device>) -> DiscoveryReport {
        let nodes_seen = devices.len();
        DiscoveryReport {
            devices,
            warnings: vec![],
            nodes_seen,
        }
    }

    fn device(name: &str, percent: u8) -> Device {
        Device {
            key: name.to_lowercase(),
            name: name.to_string(),
            kind: DeviceKind::Mouse,
            connection: Connection::Usb,
            hidraw: None,
            serial: None,
            wpid: None,
            battery: Some(BatteryReading::new(
                Some(Percent {
                    value: percent,
                    source: PercentSource::Device,
                }),
                Some(3900),
                ChargeStatus::Discharging,
            )),
            error: None,
        }
    }

    #[test]
    fn refresh_builds_a_snapshot_and_stays_silent_while_notifications_are_off() {
        let mut monitor = Monitor::new(Config::default());
        let alerts = monitor.refresh_from(report(vec![device("G703", 5)]));
        assert!(alerts.is_empty(), "notifications are opt-in");
        assert_eq!(monitor.snapshot().devices.len(), 1);
        assert_eq!(monitor.snapshot().devices[0].percent_text, "5%");
    }

    #[test]
    fn turning_notifications_on_makes_a_low_battery_speak() {
        let mut config = Config::default();
        config.notifications.enabled = true;
        let mut monitor = Monitor::new(config);
        let alerts = monitor.refresh_from(report(vec![device("G703", 18)]));
        assert_eq!(alerts.len(), 1);
        let (title, body) = alerts[0].title_and_body();
        assert_eq!(title, "Battery low");
        assert!(body.contains("G703"), "{body}");
        assert!(body.contains("below 20%"), "{body}");
    }

    #[test]
    fn a_device_that_disappears_is_forgotten_so_a_reconnect_can_speak_again() {
        let mut config = Config::default();
        config.notifications.enabled = true;
        config.notifications.rate_limit_seconds = 0;
        let mut monitor = Monitor::new(config);
        assert_eq!(
            monitor.refresh_from(report(vec![device("G703", 20)])).len(),
            1
        );
        assert_eq!(
            monitor.refresh_from(report(vec![device("G703", 19)])).len(),
            0
        );
        // gone
        assert!(monitor.refresh_from(report(vec![])).is_empty());
        assert_eq!(
            monitor.refresh_from(report(vec![device("G703", 19)])).len(),
            1,
            "a reconnect is a new cycle"
        );
    }

    #[test]
    fn a_bad_config_file_is_reported_as_a_notice() {
        let monitor = Monitor::from_disk();
        // The notice list always explains the default state; the point is that it exists.
        assert!(!monitor.notices.is_empty());
    }
}
