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
pub const APP_DIR: &str = "logi-modern";
pub const FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// How often the device list and batteries are refreshed, in seconds.
    pub refresh_seconds: u64,
    /// Start with the window hidden (tray only).
    pub start_hidden: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            refresh_seconds: 60,
            start_hidden: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    /// Off until the user asks for it.
    pub enabled: bool,
    /// Percentages worth reporting, highest first.
    pub thresholds: Vec<u8>,
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
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            show_voltage: true,
            mark_estimates: true,
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
        let dir =
            std::env::temp_dir().join(format!("logi-modern-test-{}-{name}", std::process::id()));
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
}
