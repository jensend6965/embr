//! What to persist, and how to survive a file the user edited.
//!
//! Options live in one TOML file under `$XDG_CONFIG_HOME/<app>/config.toml`. Three rules:
//!
//! * a missing file is not an error — defaults apply;
//! * a corrupt file is reported and then ignored, per field, never fatally;
//! * keys this version does not know are carried through a save untouched, so a newer or
//!   older build does not silently delete settings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Application directory name used under the XDG config home.
pub const APP_DIR: &str = "embr";
pub const FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// How often the device list and batteries are refreshed, in seconds.
    pub refresh_seconds: u64,
    /// Ask each device whether it is still there, every few seconds, while the window is open.
    /// On by default: a headset switched off should show up while the user is still looking at the
    /// window, not at the next sweep. Turning it off leaves the sweeps to notice, and costs the
    /// devices a good deal less.
    pub watch_devices: bool,
    /// What the window's close button does: `true` keeps the app running (tray only), `false`
    /// quits. On by default — a battery monitor that stops watching when its window is closed
    /// is a battery monitor that is not watching.
    pub close_to_tray: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            refresh_seconds: 60,
            close_to_tray: true,
            watch_devices: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    /// Off until the user asks for it.
    pub enabled: bool,
    /// Percentages worth reporting, highest first. The default for every device.
    pub thresholds: Vec<u8>,
    /// One level per device, keyed by the device's stable key. A device listed here watches
    /// its own level instead of `thresholds`.
    pub per_device: BTreeMap<String, u8>,
    /// How far the level must rise above a threshold before it can fire again.
    pub hysteresis_percent: u8,
    /// At most one notification per this many seconds, across all devices.
    pub rate_limit_seconds: u64,
    /// How long an acknowledged device stays quiet.
    pub snooze_minutes: u64,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            enabled: false,
            thresholds: vec![20, 10],
            per_device: BTreeMap::new(),
            hysteresis_percent: 5,
            rate_limit_seconds: 60,
            snooze_minutes: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    /// `dark`, `light` or `system`.
    pub theme: String,
    /// Show voltage next to the percentage.
    pub show_voltage: bool,
    /// Mark percentages we inferred from voltage as approximate.
    pub mark_estimates: bool,
    /// Draw the worst battery as a number in the tray icon instead of the battery glyph.
    pub tray_shows_percent: bool,
    /// The devices that get a tray icon of their own, by device key, in the order chosen. Empty —
    /// the default — means one icon that follows the device with the least charge.
    pub tray_devices: Vec<String>,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            show_voltage: true,
            mark_estimates: true,
            tray_shows_percent: false,
            tray_devices: Vec::new(),
        }
    }
}

impl Config {
    /// Give one device a tray icon of its own, or take it away again.
    ///
    /// The order the devices were chosen in is the order their icons stand in, and asking twice
    /// for the same device does not give it two icons. Devices that are not attached right now can
    /// still be chosen: the choice is the user's, not the sweep's.
    pub fn set_tray_device(&mut self, key: &str, on: bool) {
        if on {
            if !self.ui.tray_devices.iter().any(|chosen| chosen == key) {
                self.ui.tray_devices.push(key.to_string());
            }
        } else {
            self.ui.tray_devices.retain(|chosen| chosen != key);
        }
    }
}

/// The whole configuration file.
///
/// Not `Eq`: a preserved `toml::Value` is only `PartialEq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub notifications: Notifications,
    pub ui: Ui,
    /// Keys written by another version, kept so a save does not destroy them.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Config {
    /// Normalise anything a human might have typed into something usable.
    pub fn sanitise(&mut self) {
        if self.general.refresh_seconds < 5 {
            self.general.refresh_seconds = 5;
        }
        self.notifications.thresholds = {
            let mut levels: Vec<u8> = self
                .notifications
                .thresholds
                .iter()
                .copied()
                .filter(|t| (1..=99).contains(t))
                .collect();
            levels.sort_unstable();
            levels.dedup();
            levels.reverse();
            levels
        };
        if self.notifications.thresholds.is_empty() {
            self.notifications.thresholds = Notifications::default().thresholds;
        }
        // A per-device level is a percentage like any other. A level outside 1–99, or an entry
        // with no device key, is dropped so that device simply falls back to the defaults.
        self.notifications
            .per_device
            .retain(|key, level| !key.trim().is_empty() && (1..=99).contains(&*level));
        if self.notifications.hysteresis_percent > 50 {
            self.notifications.hysteresis_percent = 50;
        }
        if !matches!(self.ui.theme.as_str(), "dark" | "light" | "system") {
            self.ui.theme = "dark".to_string();
        }
    }
}

/// The file this configuration lives in.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".config")
        });
    base.join(APP_DIR).join(FILE_NAME)
}

/// Read the configuration. Never fails: on any problem the defaults are used and the reason
/// is returned so the caller can tell the user instead of hiding it.
pub fn load(path: &Path) -> (Config, Vec<String>) {
    let mut problems = Vec::new();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let mut config = Config::default();
            config.sanitise();
            return (config, problems);
        }
        Err(err) => {
            problems.push(format!("could not read {}: {err}", path.display()));
            let mut config = Config::default();
            config.sanitise();
            return (config, problems);
        }
    };
    match toml::from_str::<Config>(&text) {
        Ok(mut config) => {
            config.sanitise();
            (config, problems)
        }
        Err(err) => {
            problems.push(format!(
                "{} is not valid configuration ({err}); using defaults",
                path.display()
            ));
            let mut config = Config::default();
            config.sanitise();
            (config, problems)
        }
    }
}

/// Write the configuration atomically. Returns the reason it could not be written.
pub fn save(config: &Config, path: &Path) -> Result<(), String> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    if let Err(err) = std::fs::create_dir_all(directory) {
        return Err(format!("could not create {}: {err}", directory.display()));
    }
    let text = match toml::to_string_pretty(config) {
        Ok(text) => text,
        Err(err) => return Err(format!("could not serialise configuration: {err}")),
    };
    // A temporary file in the same directory keeps the rename on one filesystem, so the
    // user never finds a half-written config.
    let temporary = directory.join(format!(".{FILE_NAME}.tmp{}", std::process::id()));
    if let Err(err) = std::fs::write(&temporary, text) {
        return Err(format!("could not write {}: {err}", temporary.display()));
    }
    if let Err(err) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("could not replace {}: {err}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("embr-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(FILE_NAME)
    }

    #[test]
    fn defaults_are_sane_and_notifications_start_off() {
        let config = Config::default();
        assert!(
            !config.notifications.enabled,
            "battery notifications must be opt-in"
        );
        assert_eq!(config.notifications.thresholds, vec![20, 10]);
        assert_eq!(config.general.refresh_seconds, 60);
        assert!(
            config.general.close_to_tray,
            "closing the window must leave the app watching unless the user says otherwise"
        );
    }

    #[test]
    fn a_config_written_before_close_to_tray_existed_still_closes_to_the_tray() {
        let path = scratch("older");
        std::fs::write(&path, "[general]\nrefresh_seconds = 45\n").unwrap();
        let (config, problems) = load(&path);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.general.refresh_seconds, 45);
        assert!(
            config.general.close_to_tray,
            "an absent key means the default, which is to keep running"
        );
        assert!(
            config.general.watch_devices,
            "and a config written before the watch existed watches for devices by default"
        );
    }

    #[test]
    fn a_missing_file_gives_defaults_without_complaint() {
        let path = scratch("missing");
        let (config, problems) = load(&path);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn configuration_round_trips() {
        let path = scratch("roundtrip");
        let mut config = Config::default();
        config.notifications.enabled = true;
        config.notifications.thresholds = vec![35, 15];
        config.general.refresh_seconds = 30;
        save(&config, &path).unwrap();
        let (loaded, problems) = load(&path);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(loaded, config);
    }

    #[test]
    fn a_corrupt_file_reports_and_falls_back() {
        let path = scratch("corrupt");
        std::fs::write(&path, "this is not = toml [[[").unwrap();
        let (config, problems) = load(&path);
        assert_eq!(config, Config::default());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("not valid configuration"));
    }

    #[test]
    fn unknown_keys_survive_a_save() {
        let path = scratch("unknown");
        std::fs::write(
            &path,
            "battery_widget_pos = { x = 12, y = 34 }\n[general]\nrefresh_seconds = 45\n",
        )
        .unwrap();
        let (config, _) = load(&path);
        assert_eq!(config.general.refresh_seconds, 45);
        save(&config, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("battery_widget_pos"),
            "unknown keys must not be dropped:\n{text}"
        );
        assert!(text.contains("refresh_seconds = 45"));
    }

    #[test]
    fn silly_values_are_normalised_instead_of_ignored() {
        let mut config = Config::default();
        config.notifications.thresholds = vec![0, 150, 20, 20, 10];
        config.general.refresh_seconds = 1;
        config.notifications.hysteresis_percent = 200;
        config.ui.theme = "neon".to_string();
        config.sanitise();
        assert_eq!(config.notifications.thresholds, vec![20, 10]);
        assert_eq!(config.general.refresh_seconds, 5);
        assert_eq!(config.notifications.hysteresis_percent, 50);
        assert_eq!(config.ui.theme, "dark");
    }

    #[test]
    fn an_empty_threshold_list_falls_back_to_the_default() {
        let mut config = Config::default();
        config.notifications.thresholds = vec![];
        config.sanitise();
        assert_eq!(config.notifications.thresholds, vec![20, 10]);
    }

    #[test]
    fn a_per_device_level_survives_a_save_and_a_reload() {
        let path = scratch("per-device");
        let mut config = Config::default();
        config.notifications.enabled = true;
        config
            .notifications
            .per_device
            .insert("G733-0123".to_string(), 35);
        config.ui.tray_shows_percent = true;
        config
            .ui
            .tray_devices
            .extend(["G733-0123".to_string(), "hidraw8".to_string()]);
        save(&config, &path).unwrap();

        let (loaded, problems) = load(&path);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(loaded.notifications.per_device.get("G733-0123"), Some(&35));
        assert!(loaded.ui.tray_shows_percent);
        assert_eq!(
            loaded.ui.tray_devices,
            vec!["G733-0123".to_string(), "hidraw8".to_string()],
            "the order the icons were chosen in is part of the choice"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn choosing_a_device_for_the_tray_keeps_the_order_and_does_not_double_it() {
        let mut config = Config::default();
        config.set_tray_device("hidraw7", true);
        config.set_tray_device("hidraw8", true);
        config.set_tray_device("hidraw7", true); // asked for twice: still one icon
        assert_eq!(
            config.ui.tray_devices,
            vec!["hidraw7".to_string(), "hidraw8".to_string()],
            "the order they were chosen in is the order they are drawn in"
        );

        config.set_tray_device("hidraw7", false);
        assert_eq!(config.ui.tray_devices, vec!["hidraw8".to_string()]);
        config.set_tray_device("hidraw8", false);
        assert!(
            config.ui.tray_devices.is_empty(),
            "nothing chosen is the default: one icon following the worst device"
        );
    }

    #[test]
    fn an_unusable_per_device_level_is_dropped_rather_than_clamped() {
        let mut config = Config::default();
        config
            .notifications
            .per_device
            .insert("mouse".to_string(), 30);
        config
            .notifications
            .per_device
            .insert("zero".to_string(), 0);
        config
            .notifications
            .per_device
            .insert("over".to_string(), 150);
        config
            .notifications
            .per_device
            .insert("   ".to_string(), 40);
        config.sanitise();
        assert_eq!(
            config.notifications.per_device,
            BTreeMap::from([("mouse".to_string(), 30)]),
            "only the usable level is kept: the rest fall back to the defaults"
        );
    }
}
