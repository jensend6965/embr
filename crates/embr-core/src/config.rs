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

/// The shape the tray icon is drawn as.
///
/// The *set* lives here, with the configuration, because it is a user-facing vocabulary rather
/// than a drawing detail: the settings page lists these in this order, `ui.tray_style` stores them
/// by [`TrayStyle::as_str`], and the wording below is what the user chooses between. The drawing
/// itself is the app's — one glyph per style in `tray.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayStyle {
    /// A battery, coloured by the level.
    #[default]
    Battery,
    /// The same battery standing on end.
    BatteryVertical,
    /// The level itself, as digits.
    Percent,
    /// A circle that fills as the charge does.
    Ring,
    /// A battery whose fill is the level, with a bolt while it charges and a warning when the
    /// device reports a fault — the one style that says what the device is *doing* and not only
    /// how full it is.
    Status,
}

impl TrayStyle {
    /// Every style, in the order the settings page offers them.
    pub const ALL: [TrayStyle; 5] = [
        TrayStyle::Battery,
        TrayStyle::BatteryVertical,
        TrayStyle::Percent,
        TrayStyle::Ring,
        TrayStyle::Status,
    ];

    /// The name written in `ui.tray_style`. Stable: it sits in a file the user may edit.
    pub fn as_str(self) -> &'static str {
        match self {
            TrayStyle::Battery => "battery",
            TrayStyle::BatteryVertical => "battery-vertical",
            TrayStyle::Percent => "percent",
            TrayStyle::Ring => "ring",
            TrayStyle::Status => "status",
        }
    }

    /// The style a name refers to, or `None` for a name this build does not know — which is one
    /// field falling back to the default, never a configuration that fails to load.
    pub fn from_name(name: &str) -> Option<TrayStyle> {
        Self::ALL.into_iter().find(|style| style.as_str() == name)
    }

    /// What the style is called on the settings page.
    pub fn label(self) -> &'static str {
        match self {
            TrayStyle::Battery => "Battery",
            TrayStyle::BatteryVertical => "Battery, upright",
            TrayStyle::Percent => "Percentage",
            TrayStyle::Ring => "Ring",
            TrayStyle::Status => "Level and status",
        }
    }

    /// One line on what it draws, for the sentence under the picker.
    pub fn caption(self) -> &'static str {
        match self {
            TrayStyle::Battery => "the level is how much of the tube is filled",
            TrayStyle::BatteryVertical => "the same battery standing on end",
            TrayStyle::Percent => "the level itself, as digits",
            TrayStyle::Ring => "a ring that fills as the charge rises",
            TrayStyle::Status => {
                "the fill is the level, a bolt while it charges, a mark if it faults"
            }
        }
    }
}

/// The colour the tray icon's *text* is drawn in — the percentage style's digits.
///
/// It is the only colour any style has: the glyphs are line drawings in white, deliberately, so
/// that one template fits every bar and nothing in the tray shouts. The default is the level
/// itself, which is what the digits were coloured by before this was a choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayTextColor {
    /// Green, amber or red according to the level.
    #[default]
    Level,
    White,
    Green,
    Amber,
    Red,
    Blue,
    Grey,
}

/// Red, amber, green, in that order — a level's colour, and grey when there is nothing to read.
pub const LEVEL_COLORS: [(u8, u8, u8); 3] = [(242, 114, 111), (61, 220, 151), (244, 193, 80)];
/// The line every tray glyph is drawn with. White, because that is what reads on the bars this is
/// for; nothing else in an icon is coloured.
pub const WHITE: (u8, u8, u8) = (246, 247, 249);
/// The part of a shape that is not filled yet — the unfilled arc of the ring. Mid grey on purpose:
/// a dark bar would swallow anything dimmer.
pub const DIM: (u8, u8, u8) = (110, 116, 124);

impl TrayTextColor {
    /// Every choice, in the order the settings page offers them.
    pub const ALL: [TrayTextColor; 7] = [
        TrayTextColor::Level,
        TrayTextColor::White,
        TrayTextColor::Green,
        TrayTextColor::Amber,
        TrayTextColor::Red,
        TrayTextColor::Blue,
        TrayTextColor::Grey,
    ];

    /// The name written in `ui.tray_text_color`. Stable: it sits in a file the user may edit.
    pub fn as_str(self) -> &'static str {
        match self {
            TrayTextColor::Level => "level",
            TrayTextColor::White => "white",
            TrayTextColor::Green => "green",
            TrayTextColor::Amber => "amber",
            TrayTextColor::Red => "red",
            TrayTextColor::Blue => "blue",
            TrayTextColor::Grey => "grey",
        }
    }

    /// The colour a name refers to, or `None` for one this build does not know.
    pub fn from_name(name: &str) -> Option<TrayTextColor> {
        Self::ALL.into_iter().find(|color| color.as_str() == name)
    }

    /// What the choice is called on the settings page.
    pub fn label(self) -> &'static str {
        match self {
            TrayTextColor::Level => "By level",
            TrayTextColor::White => "White",
            TrayTextColor::Green => "Green",
            TrayTextColor::Amber => "Amber",
            TrayTextColor::Red => "Red",
            TrayTextColor::Blue => "Blue",
            TrayTextColor::Grey => "Grey",
        }
    }

    /// The colour itself. `Level` is not one colour, so this is the green it is most of the time —
    /// what its swatch on the settings page shows.
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            TrayTextColor::Level => LEVEL_COLORS[1],
            TrayTextColor::White => WHITE,
            TrayTextColor::Green => LEVEL_COLORS[1],
            TrayTextColor::Amber => LEVEL_COLORS[2],
            TrayTextColor::Red => LEVEL_COLORS[0],
            TrayTextColor::Blue => (106, 168, 255),
            TrayTextColor::Grey => (155, 161, 169),
        }
    }

    /// The colour to draw a reading in: by level for [`TrayTextColor::Level`], otherwise this
    /// colour whatever the level is.
    pub fn color(self, level: Option<u8>) -> (u8, u8, u8) {
        match self {
            TrayTextColor::Level => level_color(level),
            other => other.rgb(),
        }
    }
}

/// The colour a level is drawn in: red, amber, green — grey when there is nothing to read.
pub fn level_color(level: Option<u8>) -> (u8, u8, u8) {
    match level {
        None => (155, 161, 169),
        Some(percent) if percent <= 20 => LEVEL_COLORS[0],
        Some(percent) if percent <= 30 => LEVEL_COLORS[2],
        Some(_) => LEVEL_COLORS[1],
    }
}

/// Everything the tray icon is drawn from besides the reading: which shape, and — for the one
/// style that has text — what colour that text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrayLook {
    pub style: TrayStyle,
    pub text_color: TrayTextColor,
}

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
    /// Which shape the tray icon is drawn as, by [`TrayStyle::as_str`]. Kept as a string, like
    /// `theme`: a name this build does not know is one field falling back to the default, not a
    /// file that fails to load and takes the thresholds with it.
    pub tray_style: String,
    /// The colour the percentage style's digits are drawn in, by [`TrayTextColor::as_str`]. The
    /// only colour the tray icon has; the glyphs themselves are white line drawings.
    pub tray_text_color: String,
    /// The switch `tray_style` replaced — `true` meant the level as digits. Read, never written
    /// (see the comment on the field), so a configuration written before the styles existed keeps
    /// the icon it chose, and the key disappears the first time anything is saved.
    #[serde(default, skip_serializing)]
    pub tray_shows_percent: bool,
    /// The devices that get a tray icon of their own, by device key, in the order chosen. Empty —
    /// the default — means one icon that follows the device with the least charge.
    pub tray_devices: Vec<String>,
}

impl Ui {
    /// The tray icon's style, with a name this build does not know falling back to the default.
    pub fn tray_style(&self) -> TrayStyle {
        TrayStyle::from_name(&self.tray_style).unwrap_or_default()
    }

    /// The colour of that style's text, likewise.
    pub fn tray_text_color(&self) -> TrayTextColor {
        TrayTextColor::from_name(&self.tray_text_color).unwrap_or_default()
    }

    /// Both of them together: what the tray icons are drawn as, as opposed to what they say.
    pub fn tray_look(&self) -> TrayLook {
        TrayLook {
            style: self.tray_style(),
            text_color: self.tray_text_color(),
        }
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            show_voltage: true,
            mark_estimates: true,
            tray_style: TrayStyle::Battery.as_str().to_string(),
            tray_text_color: TrayTextColor::Level.as_str().to_string(),
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
        // A tray style this build does not know is one field back to the default. And a
        // configuration written before the styles existed asked for the digits with a switch
        // (`tray_shows_percent`), which is the one choice that must survive the move — but only
        // while the stored style is still the default, so a style the user picks here always wins.
        if self.ui.tray_shows_percent && self.ui.tray_style == TrayStyle::Battery.as_str() {
            self.ui.tray_style = TrayStyle::Percent.as_str().to_string();
        }
        if TrayStyle::from_name(&self.ui.tray_style).is_none() {
            self.ui.tray_style = TrayStyle::default().as_str().to_string();
        }
        if TrayTextColor::from_name(&self.ui.tray_text_color).is_none() {
            self.ui.tray_text_color = TrayTextColor::default().as_str().to_string();
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
    fn every_tray_style_round_trips_through_its_own_name() {
        for style in TrayStyle::ALL {
            assert_eq!(
                TrayStyle::from_name(style.as_str()),
                Some(style),
                "{} is written and read back as itself",
                style.as_str()
            );
            let mut config = Config::default();
            config.ui.tray_style = style.as_str().to_string();
            config.sanitise();
            assert_eq!(config.ui.tray_style(), style, "and survives a load");
        }
        assert_eq!(
            TrayStyle::from_name("battery-round"),
            None,
            "a name this build does not know is not silently some other style"
        );
        assert_eq!(TrayStyle::default(), TrayStyle::Battery);
    }

    #[test]
    fn a_style_this_build_does_not_know_falls_back_to_the_default() {
        let path = scratch("unknown-style");
        std::fs::write(&path, "[ui]\ntray_style = \"hologram\"\n").unwrap();
        let (config, problems) = load(&path);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.ui.tray_style(), TrayStyle::Battery);
    }

    #[test]
    fn every_text_colour_round_trips_through_its_own_name() {
        for color in TrayTextColor::ALL {
            assert_eq!(
                TrayTextColor::from_name(color.as_str()),
                Some(color),
                "{} is written and read back as itself",
                color.as_str()
            );
            let mut config = Config::default();
            config.ui.tray_text_color = color.as_str().to_string();
            config.sanitise();
            assert_eq!(config.ui.tray_text_color(), color, "and survives a load");
        }
        assert_eq!(
            TrayTextColor::from_name("chartreuse"),
            None,
            "a colour this build does not know is not silently some other colour"
        );
        // The default keeps the colour following the level, which is how the digits were coloured
        // before the choice existed.
        assert_eq!(TrayTextColor::default(), TrayTextColor::Level);
    }

    #[test]
    fn a_text_colour_this_build_does_not_know_falls_back_to_the_level() {
        let path = scratch("unknown-colour");
        std::fs::write(
            &path,
            "[ui]\ntray_style = \"percent\"\ntray_text_color = \"chartreuse\"\n",
        )
        .unwrap();
        let (config, problems) = load(&path);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.ui.tray_text_color(), TrayTextColor::Level);
        assert_eq!(config.ui.tray_look().style, TrayStyle::Percent);
    }

    #[test]
    fn the_look_is_the_two_choices_together() {
        let mut config = Config::default();
        config.ui.tray_style = TrayStyle::Ring.as_str().to_string();
        config.ui.tray_text_color = TrayTextColor::Blue.as_str().to_string();
        config.sanitise();
        assert_eq!(
            config.ui.tray_look(),
            TrayLook {
                style: TrayStyle::Ring,
                text_color: TrayTextColor::Blue,
            }
        );
    }

    #[test]
    fn the_old_switch_for_the_digits_still_chooses_the_digits() {
        // Jensen's own file, before the styles existed: the icon he chose must not be lost just
        // because the key that asked for it was replaced.
        let path = scratch("legacy-percent");
        std::fs::write(&path, "[ui]\ntheme = \"dark\"\ntray_shows_percent = true\n").unwrap();
        let (config, problems) = load(&path);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.ui.tray_style(), TrayStyle::Percent);

        // Read once, never written: the first save drops the key, and after that only the style
        // in the file decides what the icon looks like.
        save(&config, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("tray_shows_percent"),
            "the replaced key must not be written back:\n{text}"
        );
        assert!(text.contains("tray_style = \"percent\""), "{text}");

        // And a style chosen here wins over the leftover switch, whatever the file says.
        let mut chosen = config.clone();
        chosen.ui.tray_shows_percent = true;
        chosen.ui.tray_style = "ring".to_string();
        chosen.sanitise();
        assert_eq!(chosen.ui.tray_style(), TrayStyle::Ring);
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
        config.ui.tray_style = "ring".to_string();
        config
            .ui
            .tray_devices
            .extend(["G733-0123".to_string(), "hidraw8".to_string()]);
        save(&config, &path).unwrap();

        let (loaded, problems) = load(&path);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(loaded.notifications.per_device.get("G733-0123"), Some(&35));
        assert_eq!(
            loaded.ui.tray_style(),
            TrayStyle::Ring,
            "the style the user chose is the style that comes back"
        );
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
