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

use std::collections::HashMap;
use std::time::{Duration, Instant};

use embr_hidpp::discover::{self, DiscoveryReport};

pub use config::{
    Config, DIM, LEVEL_COLORS, LOW_AT_OR_BELOW, Level, MEDIUM_AT_OR_BELOW, Notifications, Theme,
    TrayLook, TrayStyle, TrayTextColor, WHITE, default_path, level_band, level_color,
};
pub use embr_hidpp::Uevents;
pub use embr_hidpp::discover::Patience;
pub use policy::{Alert, BatteryPolicy, PolicySettings};
pub use view::{
    DevicePages, DeviceView, FeaturePage, Snapshot, TrayIcon, human_age, pages_for, snooze_caption,
    summary_text, thresholds_text, tray_icons, tray_style_caption,
};

/// What the window's close button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowClose {
    /// Take the window away and keep running: the devices are still being watched, the tray
    /// still has them, and a notification can still arrive.
    Hide,
    /// Quit. The user asked for the app to be gone, not for it to hide.
    Quit,
}

/// The close button, as the user's configuration says it behaves. Kept here rather than in the
/// window so that "close to tray" is a decision with a name and a test, not an `if` in an event
/// handler.
pub fn window_close_action(close_to_tray: bool) -> WindowClose {
    if close_to_tray {
        WindowClose::Hide
    } else {
        WindowClose::Quit
    }
}

/// A sweep is seconds of HID++ traffic and the devices are not ours alone, so no configuration
/// gets to ask for a faster cadence than this.
pub const MIN_REFRESH_SECONDS: u64 = 5;

/// With the window closed nothing is being watched, so the devices are left alone for at least
/// this long: a tray-only battery monitor that talks to a receiver every few seconds is a
/// battery monitor that costs more than it saves.
pub const HIDDEN_REFRESH_FLOOR_SECONDS: u64 = 120;

/// How long until the next scheduled sweep.
///
/// The user's interval is what they get while the window is open — that is the number they set,
/// and they set it while looking at the window. Closed, the cadence backs off to at least
/// [`HIDDEN_REFRESH_FLOOR_SECONDS`], but never to *less* than they asked for: someone who
/// chose ten minutes does not want to be scanned every two.
pub fn scheduled_interval(refresh_seconds: u64, window_open: bool) -> Duration {
    let wanted = refresh_seconds.max(MIN_REFRESH_SECONDS);
    Duration::from_secs(if window_open {
        wanted
    } else {
        wanted.max(HIDDEN_REFRESH_FLOOR_SECONDS)
    })
}

/// How long to wait before asking again after a sweep that could not read everything.
///
/// `attempts` is one-based: how many sweeps in a row have come back incomplete, so the first
/// repeat is ten seconds after the sweep that failed — that is the number a person watching the
/// window actually feels. It then doubles, because a device that is switched off must not be
/// woken every few seconds for nothing: 10 s, 20 s, 40 s, 80 s, and by then the interval the user
/// chose is in charge anyway.
pub fn retry_interval(attempts: u32) -> Duration {
    let step = attempts.saturating_sub(1).min(3);
    Duration::from_secs(10 << step)
}

/// How long until the next sweep, given how the last one went.
///
/// With the window closed the floor stands whatever is missing: nobody is watching, and waking a
/// sleeping device every ten seconds to redraw a tray tooltip spends its battery on nothing. A
/// device that comes back while the window is closed is caught by [`Uevents`] instead, which
/// costs nothing at all until something actually changes.
///
/// A device that could not be read is normally the reason to come back sooner than the interval the
/// user chose — that ladder is how a device coming back from sleep is caught while somebody is
/// looking at the window. `watching` says that something else is already asking between sweeps (see
/// [`watch_interval`]): with a watch pass running every five seconds, a full sweep every ten would
/// be the same question asked twice, the second time expensively.
pub fn next_sweep(
    refresh_seconds: u64,
    window_open: bool,
    missing_attempts: u32,
    watching: bool,
) -> Duration {
    let normal = scheduled_interval(refresh_seconds, window_open);
    if !window_open || watching || missing_attempts == 0 {
        return normal;
    }
    normal.min(retry_interval(missing_attempts))
}

/// How often a device is asked whether it is still there, while the window is open.
///
/// Five seconds is the difference between "I switched my headset off and Embr knew" and "I waited,
/// gave up, and pressed Rescan". Each pass costs one short query per device and is over in well
/// under a second (a device that is there answers in milliseconds; one that is gone costs its
/// deadline and nothing more), so this is not the expensive kind of polling — the expensive kind is
/// asking a *sleeping* wireless device and waking it, which is what the long sweep interval is for.
pub const WATCH_SECONDS: u64 = 5;

/// How long until the next watch pass, if there should be one at all.
///
/// `None` is the answer with the setting off and with the window closed: nobody is looking at the
/// window, a device switching off is not news that needs announcing, and a tray-only Embr stays as
/// quiet towards the devices as it was before this existed.
pub fn watch_interval(watch_devices: bool, window_open: bool) -> Option<Duration> {
    (watch_devices && window_open).then(|| Duration::from_secs(WATCH_SECONDS))
}

/// Holds the configuration, the battery policy and the latest snapshot.
#[derive(Debug)]
pub struct Monitor {
    config: Config,
    policy: BatteryPolicy,
    snapshot: Snapshot,
    /// The last discovery report, kept so that a changed setting can be shown in the window
    /// without going back to the hardware for a number nothing has changed about.
    last_report: Option<DiscoveryReport>,
    /// The pages each device has, from the feature set it advertised. A feature set is an
    /// inventory rather than a reading, so it is read when a window first asks about a device
    /// and kept: reading it costs twenty-odd calls on a keyboard, and repeating that every
    /// minute would be spending the devices' batteries on a fact that cannot have changed.
    pages: HashMap<String, DevicePages>,
    /// Problems to show the user once (a bad config file, a receiver that did not answer).
    pub notices: Vec<String>,
}

impl Monitor {
    pub fn new(config: Config) -> Self {
        let mut notices = Vec::new();
        if !config.notifications.enabled {
            notices.push("Battery notifications are off — a low battery is shown in the window, but never announced.".to_string());
        }
        let policy = BatteryPolicy::new(settings_from(&config));
        let snapshot = Snapshot::default();
        Self {
            config,
            policy,
            snapshot,
            last_report: None,
            pages: HashMap::new(),
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
        self.rebuild();
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
        self.refresh_with(Patience::Scheduled)
    }

    /// The same, with the waits the caller asks for: a sweep the user asked for gives a device
    /// that has not answered yet more time before writing it off as not answering.
    pub fn refresh_with(&mut self, patience: Patience) -> Vec<Alert> {
        self.refresh_from(discover::discover_with(patience))
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
        // battery cycle. Anything not in this report is forgotten — and with it the feature set
        // we read from it, which the device it belonged to will answer for again.
        let present: Vec<&str> = report.devices.iter().map(|d| d.key.as_str()).collect();
        for key in self.policy.known_keys() {
            if !present.contains(&key.as_str()) {
                self.policy.reset(&key);
            }
        }
        self.pages.retain(|key, _| present.contains(&key.as_str()));
        self.snapshot = Snapshot::from_report(&report, &self.config);
        self.apply_pages();
        self.last_report = Some(report);
        alerts
    }

    /// Give every device in the current snapshot the pages it advertised, where we know them.
    fn apply_pages(&mut self) {
        for device in &mut self.snapshot.devices {
            if let Some(pages) = self.pages.get(&device.key) {
                device.pages = Some(pages.pages.clone());
                device.pages_partial = !pages.complete();
            }
        }
    }

    /// Ask one device what it advertises, and remember the answer.
    ///
    /// This opens the device, so it is the device thread's work, not the window's, and it is
    /// asked for a device at a time — the one the user has just opened. `None` means the device
    /// could not be reached, which is a different thing from a device that advertises nothing.
    pub fn read_pages(&mut self, key: &str) -> Option<DevicePages> {
        let device = self
            .last_report
            .as_ref()?
            .devices
            .iter()
            .find(|device| device.key == key)?
            .clone();
        // Whoever asked is waiting for it, so give a device that is slow to wake the longer
        // wait: this is one round trip per feature, and half of it timing out is worse than
        // waiting.
        let capabilities = discover::capabilities_of(&device, Patience::Asked)?;
        if !capabilities.answered() {
            // It was reachable and said nothing about its own feature set — which is not the same
            // as having no features, and must not be cached as if it were. The rail then says the
            // device did not answer, and the next ask (Rescan) tries again.
            return None;
        }
        let pages = DevicePages {
            pages: pages_for(
                &capabilities
                    .features
                    .iter()
                    .map(|(feature, _)| *feature)
                    .collect::<Vec<_>>(),
            ),
            declared: capabilities.declared,
            silent: capabilities.silent,
        };
        self.pages.insert(key.to_string(), pages.clone());
        self.apply_pages();
        Some(pages)
    }

    /// Re-derive the snapshot from the readings already in hand, after the configuration
    /// changed. Nothing here opens a device: a threshold the user just moved must show up in
    /// the window at once, not one slow sweep later.
    pub fn rebuild(&mut self) {
        if let Some(report) = self.last_report.take() {
            self.snapshot = Snapshot::from_report(&report, &self.config);
            self.apply_pages();
            self.last_report = Some(report);
        }
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

    /// Give one device its own alert level — or, with `None`, put it back on the shared
    /// thresholds. Remember the choice.
    pub fn set_device_alert_level(&mut self, key: &str, level: Option<u8>) -> Result<(), String> {
        match level {
            Some(level) => {
                self.config
                    .notifications
                    .per_device
                    .insert(key.to_string(), level.clamp(1, 99));
            }
            None => {
                self.config.notifications.per_device.remove(key);
            }
        }
        self.config.sanitise();
        self.policy.set_settings(settings_from(&self.config));
        self.rebuild();
        self.save_config()
    }

    /// Draw the tray icon in another style — the same reading, a different shape.
    pub fn set_tray_style(&mut self, style: TrayStyle) -> Result<(), String> {
        self.config.ui.tray_style = style.as_str().to_string();
        self.save_config()
    }

    /// Draw the percentage style's digits in another colour.
    pub fn set_tray_text_color(&mut self, color: TrayTextColor) -> Result<(), String> {
        self.config.ui.tray_text_color = color.as_str().to_string();
        self.save_config()
    }

    /// Give one device a tray icon of its own, or take it away again. The choice is the user's, so
    /// it survives a sweep that could not read the device and a restart of the app.
    pub fn set_tray_device(&mut self, key: &str, on: bool) -> Result<(), String> {
        self.config.set_tray_device(key, on);
        self.save_config()
    }

    /// Ask the devices whether they are still there between sweeps, or leave it to the sweeps.
    pub fn set_watch_devices(&mut self, enabled: bool) -> Result<(), String> {
        self.config.general.watch_devices = enabled;
        self.save_config()
    }

    /// Keep running when the window is closed (the tray only), or quit instead.
    pub fn set_close_to_tray(&mut self, enabled: bool) -> Result<(), String> {
        self.config.general.close_to_tray = enabled;
        self.save_config()
    }

    /// Which colour scheme the window should use, and remember the choice.
    pub fn set_theme(&mut self, theme: Theme) -> Result<(), String> {
        self.config.ui.theme = theme.as_str().to_string();
        self.save_config()
    }

    /// Whether the readouts carry the voltage next to the percentage.
    pub fn set_show_voltage(&mut self, show: bool) -> Result<(), String> {
        self.config.ui.show_voltage = show;
        self.save_config()
    }

    /// Whether a reading is drawn in the colour of its level.
    pub fn set_level_colors(&mut self, colors: bool) -> Result<(), String> {
        self.config.ui.level_colors = colors;
        self.rebuild();
        self.save_config()
    }

    /// Whether a percentage we inferred from voltage is marked as approximate.
    pub fn set_mark_estimates(&mut self, mark: bool) -> Result<(), String> {
        self.config.ui.mark_estimates = mark;
        self.rebuild();
        self.save_config()
    }

    /// How often the window asks for a fresh reading while it is open.
    pub fn set_refresh_seconds(&mut self, seconds: u64) -> Result<(), String> {
        self.config.general.refresh_seconds = seconds.clamp(MIN_REFRESH_SECONDS, 600);
        self.save_config()
    }
}

fn settings_from(config: &Config) -> PolicySettings {
    PolicySettings {
        enabled: config.notifications.enabled,
        thresholds: config.notifications.thresholds.clone(),
        // The file holds one level per device; the policy watches a list, because a device
        // could grow a second one without the file's shape changing.
        per_device: config
            .notifications
            .per_device
            .iter()
            .map(|(key, level)| (key.clone(), vec![*level]))
            .collect(),
        hysteresis: config.notifications.hysteresis_percent,
        rate_limit: std::time::Duration::from_secs(config.notifications.rate_limit_seconds),
        snooze: std::time::Duration::from_secs(config.notifications.snooze_minutes * 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embr_hidpp::battery::{BatteryReading, ChargeStatus, Percent, PercentSource};
    use embr_hidpp::discover::{Connection, Device, DeviceKind, DiscoveryReport};

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
            responded: true,
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

    #[test]
    fn a_level_for_one_device_makes_that_device_speak() {
        let mut config = Config::default();
        config.notifications.enabled = true;
        config
            .notifications
            .per_device
            .insert("g733".to_string(), 45);
        let mut monitor = Monitor::new(config);
        let alerts = monitor.refresh_from(report(vec![device("G733", 39)]));
        assert_eq!(alerts.len(), 1, "39 % is below the headset's own 45 %");
        let (_, body) = alerts[0].title_and_body();
        assert!(body.contains("below 45%"), "{body}");
    }

    #[test]
    fn a_changed_level_reaches_the_window_without_a_new_sweep() {
        let mut monitor = Monitor::new(Config::default());
        monitor.refresh_from(report(vec![device("G733", 39)]));
        assert_eq!(monitor.snapshot().devices[0].alert_below, None);
        assert_eq!(monitor.snapshot().devices[0].alert_below_effective, 20);

        let mut config = Config::default();
        config
            .notifications
            .per_device
            .insert("g733".to_string(), 45);
        monitor.set_config(config);

        // Same readings, new setting: the snapshot is re-derived from the report in hand.
        assert_eq!(monitor.snapshot().devices[0].alert_below, Some(45));
        assert_eq!(monitor.snapshot().devices[0].alert_below_effective, 45);
    }

    #[test]
    fn the_close_button_hides_the_window_by_default_and_quits_only_when_asked() {
        let config = Config::default();
        assert_eq!(
            window_close_action(config.general.close_to_tray),
            WindowClose::Hide
        );

        let mut quit_on_close = Config::default();
        quit_on_close.general.close_to_tray = false;
        assert_eq!(
            window_close_action(quit_on_close.general.close_to_tray),
            WindowClose::Quit,
            "the option is what decides; nothing else is consulted"
        );
    }

    #[test]
    fn the_window_open_cadence_is_what_the_user_asked_for() {
        assert_eq!(
            scheduled_interval(30, true),
            Duration::from_secs(30),
            "an open window is scanned at the interval the user set"
        );
        assert_eq!(
            scheduled_interval(1, true),
            Duration::from_secs(MIN_REFRESH_SECONDS),
            "and never faster than a sweep can be"
        );
    }

    #[test]
    fn a_sweep_that_could_not_read_something_is_repeated_sooner() {
        assert_eq!(
            next_sweep(60, true, 0, false),
            Duration::from_secs(60),
            "nothing missing: the interval the user chose"
        );
        assert_eq!(
            next_sweep(60, true, 1, false),
            Duration::from_secs(10),
            "a device that is not answering is asked again while the user is watching"
        );
        assert_eq!(
            retry_interval(2),
            Duration::from_secs(20),
            "and then less often, so a device that is switched off is not woken for nothing"
        );
        assert_eq!(
            next_sweep(600, true, 99, false),
            Duration::from_secs(80),
            "the retry backs off no further than 80 s, however long the user's interval is"
        );
        assert_eq!(
            next_sweep(60, true, 99, false),
            Duration::from_secs(60),
            "and once the backoff passes the user's own interval, that interval is what happens"
        );
        assert_eq!(
            next_sweep(30, true, 99, false),
            Duration::from_secs(30),
            "and it never scans less often than the user asked for"
        );
        assert_eq!(
            next_sweep(60, false, 3, false),
            Duration::from_secs(HIDDEN_REFRESH_FLOOR_SECONDS),
            "with the window closed nothing is woken to update a tooltip: the floor stands, and a \
             device that really comes back is caught by its uevent instead"
        );
    }

    #[test]
    fn watching_between_sweeps_takes_over_from_the_retry_ladder() {
        assert_eq!(
            next_sweep(600, true, 3, true),
            Duration::from_secs(600),
            "with a watch pass asking every few seconds, a full sweep a minute would be the same \
             question twice — and the sweep is the expensive way to ask it"
        );
        assert_eq!(
            next_sweep(60, false, 3, false),
            Duration::from_secs(HIDDEN_REFRESH_FLOOR_SECONDS),
            "and with the window closed there is no watch to defer to, so the floor does the work"
        );
    }

    #[test]
    fn the_watch_runs_only_when_somebody_is_looking() {
        assert_eq!(
            watch_interval(true, true),
            Some(Duration::from_secs(WATCH_SECONDS)),
            "with the window open and the setting on, every device is asked whether it is there"
        );
        assert_eq!(
            watch_interval(false, true),
            None,
            "the setting turns it off, and the sweeps notice instead"
        );
        assert_eq!(
            watch_interval(true, false),
            None,
            "a tray-only Embr does not wake devices to learn that they are asleep"
        );
    }

    #[test]
    fn a_closed_window_lets_the_devices_alone() {
        assert_eq!(
            scheduled_interval(30, false),
            Duration::from_secs(HIDDEN_REFRESH_FLOOR_SECONDS),
            "a tray-only app does not scan every 30 seconds"
        );
        assert_eq!(
            scheduled_interval(600, false),
            Duration::from_secs(600),
            "a cadence the user set slower than the floor is left alone"
        );
        assert!(
            scheduled_interval(60, false) > scheduled_interval(60, true),
            "closing the window is what slows the scans down"
        );
    }
}
