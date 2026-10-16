//! The view model: everything the UI needs, computed without a widget in sight.
//!
//! Widget code should format nothing and decide nothing. It takes a [`Snapshot`] and draws
//! it, which is what makes the interesting parts testable without a display.

use std::time::SystemTime;

use embr_hidpp::discover::{Device, DiscoveryReport};

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
    /// The alert level the user set for this device alone, if any. It replaces the shared
    /// thresholds for this device.
    pub alert_below: Option<u8>,
    /// The level this device's alert actually uses: its own, or the highest shared one.
    pub alert_below_effective: u8,
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
        let alert_below = config.notifications.per_device.get(&device.key).copied();
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
            alert_below,
            // A level for a device that is not in the config yet, or a config with no
            // thresholds at all, still has to show something a spin box can hold.
            alert_below_effective: alert_below
                .or_else(|| config.notifications.thresholds.first().copied())
                .unwrap_or(20),
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
    /// Whether some device could not be read at all — the *Not answering* case, not a device that
    /// answered and simply has no battery to report (there is nothing to wait for in that one).
    pub fn incomplete(&self) -> bool {
        self.devices.iter().any(|device| device.error.is_some())
    }

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

/// One icon the tray should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayIcon {
    /// The device this icon is about, by key; `None` when it follows the worst device.
    pub key: Option<String>,
    /// What the tooltip is titled with.
    pub title: String,
    /// What the tooltip says.
    pub detail: String,
    /// The level to draw — as the number itself, as the colour of the battery glyph, or as how
    /// full the icon is.
    pub level: Option<u8>,
    /// The device is on external power. The `status` style draws a bolt for this, and it is the
    /// one thing about a battery that the level alone cannot say.
    pub charging: bool,
    /// The device itself reports a broken or overheated battery: nothing about the level is worth
    /// drawing until that is dealt with.
    pub faulty: bool,
}

impl TrayIcon {
    /// The id this icon registers as.
    ///
    /// `embr` for the icon that follows the worst device, `embr-<key>` for one about a particular
    /// device. A bar that draws only the items it has pinned (Noctalia does) matches on exactly
    /// this string, so it has to be stable and predictable rather than pretty.
    pub fn id(&self) -> String {
        match &self.key {
            None => "embr".to_string(),
            Some(key) => format!("embr-{}", id_safe(key)),
        }
    }
}

/// A device key in a shape a StatusNotifierItem id can carry: alphanumerics kept, everything else
/// turned into a dash (serial numbers and `hidraw0` come through untouched, `0005:046D:B025` does
/// not).
fn id_safe(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Which tray icons the configuration asks for, and what each one says.
///
/// Nothing chosen means one icon that follows the worst device — the app's own face. Chosen devices
/// each get an icon of their own, in the order they were chosen, like the separate readouts a
/// monitoring overlay puts on screen.
///
/// A chosen device that has no reading, or is not attached at all, still gets its icon: an icon
/// that came and went as a mouse went to sleep would be worse than one that says it has nothing to
/// show, and the tooltip says which of the two it is.
pub fn tray_icons(snapshot: &Snapshot, chosen: &[String]) -> Vec<TrayIcon> {
    if chosen.is_empty() {
        let worst = snapshot.lowest();
        return vec![TrayIcon {
            key: None,
            title: "Embr".to_string(),
            detail: snapshot.tray_tooltip(),
            level: worst.and_then(|device| device.percent),
            // The state comes from the device the icon is about, like the level does: an icon
            // that said "charging" while showing the mouse's charge would be worse than useless.
            charging: worst.is_some_and(|device| device.charging),
            faulty: worst.is_some_and(|device| device.faulty),
        }];
    }

    chosen
        .iter()
        .map(|key| {
            let device = snapshot.devices.iter().find(|device| device.key == *key);
            match device {
                Some(device) => TrayIcon {
                    key: Some(key.clone()),
                    title: device.name.clone(),
                    detail: if device.percent.is_some() {
                        format!(
                            "{} — {} ({})",
                            device.name, device.percent_text, device.status
                        )
                    } else {
                        format!("{} — {}", device.name, device.status)
                    },
                    level: device.percent,
                    charging: device.charging,
                    faulty: device.faulty,
                },
                None => TrayIcon {
                    key: Some(key.clone()),
                    title: "Embr".to_string(),
                    detail: "Not attached: this device is not here right now.".to_string(),
                    level: None,
                    charging: false,
                    faulty: false,
                },
            }
        })
        .collect()
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
    use embr_hidpp::battery::{BatteryReading, ChargeStatus, Percent, PercentSource};
    use embr_hidpp::discover::{Connection, DeviceKind};
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

    fn report(devices: Vec<Device>) -> DiscoveryReport {
        DiscoveryReport {
            nodes_seen: devices.len(),
            devices,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn with_nothing_chosen_the_icon_follows_the_worst_device() {
        let snapshot = Snapshot::from_report(
            &report(vec![
                device("Mouse", Some((80, PercentSource::Device))),
                device("Keyboard", Some((41, PercentSource::Device))),
                device("Headset", None),
            ]),
            &config(),
        );

        let icons = tray_icons(&snapshot, &[]);
        assert_eq!(
            icons.len(),
            1,
            "the default is one icon, not one per device"
        );
        assert_eq!(
            icons[0].id(),
            "embr",
            "the id that was pinnable before stays"
        );
        assert_eq!(
            icons[0].level,
            Some(41),
            "it follows the worst battery, not the first row"
        );
        assert!(
            icons[0].detail.contains("Keyboard"),
            "and the tooltip still lists every device: {}",
            icons[0].detail
        );
    }

    #[test]
    fn chosen_devices_each_get_an_icon_of_their_own_in_the_order_chosen() {
        let snapshot = Snapshot::from_report(
            &report(vec![
                device("Mouse", Some((80, PercentSource::Device))),
                device("Keyboard", Some((41, PercentSource::Device))),
            ]),
            &config(),
        );

        let chosen = vec!["keyboard".to_string(), "mouse".to_string()];
        let icons = tray_icons(&snapshot, &chosen);
        assert_eq!(icons.len(), 2, "one icon per chosen device");
        assert_eq!(icons[0].id(), "embr-keyboard");
        assert_eq!(icons[0].level, Some(41), "each icon shows its own device");
        assert_eq!(icons[1].id(), "embr-mouse");
        assert_eq!(icons[1].level, Some(80));
        assert_eq!(
            icons[1].title, "Mouse",
            "the tooltip names the device the icon is about"
        );
    }

    #[test]
    fn a_chosen_device_that_has_gone_quiet_keeps_its_icon() {
        let mut quiet = device("Headset", None);
        quiet.error = Some("no reply".to_string());
        let snapshot = Snapshot::from_report(&report(vec![quiet]), &config());

        // One that is attached but saying nothing, and one that is not attached at all: both keep
        // their icon, because an icon that comes and goes with a sleeping device is worse than one
        // that says what it does not know.
        let chosen = vec!["headset".to_string(), "ghost".to_string()];
        let icons = tray_icons(&snapshot, &chosen);
        assert_eq!(icons.len(), 2);
        assert_eq!(icons[0].id(), "embr-headset");
        assert_eq!(icons[0].level, None);
        assert!(
            icons[0].detail.contains("Headset") && icons[0].detail.contains("Connected"),
            "it says the device is there: {}",
            icons[0].detail
        );
        assert_eq!(icons[1].id(), "embr-ghost");
        assert_eq!(icons[1].level, None);
        assert!(
            icons[1].detail.contains("Not attached"),
            "and this one says it is not: {}",
            icons[1].detail
        );
    }

    #[test]
    fn a_key_that_is_not_a_clean_identifier_still_gives_a_predictable_id() {
        let icon = TrayIcon {
            key: Some("0005:046D:B025".to_string()),
            title: String::new(),
            detail: String::new(),
            level: None,
            charging: false,
            faulty: false,
        };
        assert_eq!(
            icon.id(),
            "embr-0005-046D-B025",
            "an id a bar can be asked to pin, whatever the kernel called the device"
        );
    }

    #[test]
    fn an_icon_carries_the_state_of_the_device_it_is_about() {
        // The status style draws a bolt for this: if the state came from anywhere but the device
        // the level came from, the icon would contradict itself.
        let mut charging = device("Keyboard", Some((88, PercentSource::Device)));
        charging.battery = Some(BatteryReading::new(
            Some(Percent {
                value: 88,
                source: PercentSource::Device,
            }),
            Some(4100),
            ChargeStatus::Recharging,
        ));
        let snapshot = Snapshot::from_report(
            &report(vec![
                device("Mouse", Some((40, PercentSource::Device))),
                charging,
            ]),
            &config(),
        );

        let worst = tray_icons(&snapshot, &[]);
        assert_eq!(worst[0].level, Some(40), "the worst battery is the mouse");
        assert!(
            !worst[0].charging,
            "and the icon does not borrow the keyboard's state"
        );

        let chosen = tray_icons(&snapshot, &["keyboard".to_string()]);
        assert_eq!(chosen[0].level, Some(88));
        assert!(
            chosen[0].charging,
            "the keyboard's own icon says it is on external power"
        );

        // A device that calls its own battery broken says so in the icon, whatever the level.
        let mut faulty = device("Fault", Some((50, PercentSource::Device)));
        faulty.battery = Some(BatteryReading::new(
            Some(Percent {
                value: 50,
                source: PercentSource::Device,
            }),
            Some(3900),
            ChargeStatus::InvalidBattery,
        ));
        let broken = Snapshot::from_report(&report(vec![faulty]), &config());
        let icon = tray_icons(&broken, &[]);
        assert!(icon[0].faulty, "a fault is not a level");
        assert!(!icon[0].charging);
    }

    #[test]
    fn a_sweep_that_could_not_read_a_device_is_incomplete() {
        let config = config();

        // Nothing has been read yet, and nothing is missing.
        assert!(!Snapshot::default().incomplete());

        // A device that answered and has no battery to report is not something to wait for:
        // asking it again in ten seconds would get the same answer.
        assert!(
            !Snapshot::from_report(&report(vec![device("Headset", None)]), &config).incomplete()
        );

        // A device that could not be read at all is exactly the one worth asking again.
        let mut silent = device("Mouse", None);
        silent.responded = false;
        silent.error = Some("no reply".to_string());
        let snapshot = Snapshot::from_report(&report(vec![silent]), &config);
        assert!(snapshot.incomplete());
        assert_eq!(snapshot.devices[0].error.as_deref(), Some("no reply"));

        // And one bad device among good ones is still an incomplete sweep.
        let mut second_silent = device("Keyboard", Some((90, PercentSource::Device)));
        second_silent.error = Some("no reply".to_string());
        let mixed = Snapshot::from_report(
            &report(vec![
                device("Mouse", Some((80, PercentSource::Device))),
                second_silent,
            ]),
            &config,
        );
        assert!(mixed.incomplete());
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
    fn the_tooltip_lists_every_device_and_says_when_there_is_no_reading() {
        let report = DiscoveryReport {
            devices: vec![
                device("G703 LIGHTSPEED", Some((88, PercentSource::Estimated))),
                device("G915 TKL", None),
            ],
            warnings: vec![],
            nodes_seen: 9,
        };
        let snapshot = Snapshot::from_report(&report, &config());
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
    fn the_lowest_device_is_the_one_the_tray_icon_reports() {
        let report = DiscoveryReport {
            devices: vec![
                device("Full", Some((90, PercentSource::Device))),
                device("Low", Some((7, PercentSource::Estimated))),
                device("Unknown", None),
            ],
            warnings: vec![],
            nodes_seen: 3,
        };
        let snapshot = Snapshot::from_report(&report, &config());
        let lowest = snapshot.lowest().expect("one device has a level");
        assert_eq!(lowest.name, "Low");
        assert_eq!(lowest.percent, Some(7));
    }

    #[test]
    fn a_devices_own_alert_level_is_carried_into_the_view() {
        let mut config = config();
        config.notifications.per_device.insert("a".to_string(), 35);
        let own = DeviceView::from_device(&device("A", Some((80, PercentSource::Device))), &config);
        assert_eq!(own.alert_below, Some(35));
        assert_eq!(own.alert_below_effective, 35);

        let shared =
            DeviceView::from_device(&device("B", Some((80, PercentSource::Device))), &config);
        assert_eq!(shared.alert_below, None);
        assert_eq!(
            shared.alert_below_effective, 20,
            "without a level of its own, the highest shared threshold is what the row shows"
        );
    }

    #[test]
    fn an_empty_snapshot_says_so_instead_of_showing_zero_devices() {
        let snapshot = Snapshot::default();
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
