//! When to tell the user their battery is low — and, more importantly, when not to.
//!
//! A battery monitor that notifies too eagerly is worse than none, so all the judgement
//! lives here, in one place, with an injected clock so it can be tested without waiting:
//!
//! * only while **discharging** — a device on the charger is not interesting;
//! * **once per threshold per discharge cycle** — crossing 20 % twice in one discharge, or
//!   reconnecting the same device, does not notify again;
//! * **hysteresis** — after firing at 20 % the threshold re-arms only once the level has
//!   come back above `20 + margin`, so charge/discharge flapping stays quiet;
//! * **one broken battery is not a low battery** — a device reporting an invalid or
//!   thermally failed battery produces no alert at all;
//! * **a global rate limit** — a receiver with five devices cannot produce five
//!   notifications in one second;
//! * **snooze** — the user acknowledged this device, so leave it alone for a while.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use embr_hidpp::{BatteryReading, ChargeStatus};

/// The user's wishes, as the policy needs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySettings {
    pub enabled: bool,
    /// Thresholds to watch, any order (they are sorted internally).
    pub thresholds: Vec<u8>,
    /// Percent the level must recover by before a fired threshold can fire again.
    pub hysteresis: u8,
    /// At most one alert per this window across all devices.
    pub rate_limit: Duration,
    /// How long a snoozed device stays quiet.
    pub snooze: Duration,
}

impl Default for PolicySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            thresholds: vec![20, 10],
            hysteresis: 5,
            rate_limit: Duration::from_secs(60),
            snooze: Duration::from_secs(30 * 60),
        }
    }
}

/// Something worth telling the user about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alert {
    /// A device fell to (or below) a watched threshold while discharging.
    Low {
        key: String,
        name: String,
        threshold: u8,
        percent: u8,
        estimated: bool,
    },
    /// A device finished charging: worth exactly one message per charge cycle.
    Charged {
        key: String,
        name: String,
        percent: u8,
    },
}

impl Alert {
    /// The device this alert is about, for snoozing and de-duplication.
    pub fn key(&self) -> &str {
        match self {
            Self::Low { key, .. } | Self::Charged { key, .. } => key,
        }
    }

    /// Notification title and body.
    pub fn title_and_body(&self) -> (String, String) {
        match self {
            Self::Low {
                name,
                threshold,
                percent,
                estimated,
                ..
            } => {
                let mark = if *estimated { "about " } else { "" };
                (
                    "Battery low".to_string(),
                    format!("{name}: {mark}{percent}% (below {threshold}%), discharging"),
                )
            }
            Self::Charged { name, percent, .. } => (
                "Battery charged".to_string(),
                format!("{name}: {percent}% — ready to unplug"),
            ),
        }
    }
}

/// What we remember about one device.
#[derive(Debug, Clone, Default)]
struct DeviceState {
    /// Set once the device has been seen, so an empty `armed` list means "everything has
    /// already been reported" rather than "we have not started yet".
    initialised: bool,
    /// Thresholds still armed for this discharge cycle.
    armed: Vec<u8>,
    /// Set while the device is on external power, so the cycle is not restarted on every
    /// single charging reading.
    charging: bool,
    /// A `Charged` message has been sent for the current charge cycle.
    charged_sent: bool,
    snoozed_until: Option<Instant>,
}

/// The battery policy: feed it readings, get back the alerts worth showing.
#[derive(Debug)]
pub struct BatteryPolicy {
    settings: PolicySettings,
    devices: HashMap<String, DeviceState>,
    last_alert: Option<Instant>,
}

impl BatteryPolicy {
    pub fn new(settings: PolicySettings) -> Self {
        let mut settings = settings;
        settings.thresholds = normalise_thresholds(&settings.thresholds);
        Self {
            settings,
            devices: HashMap::new(),
            last_alert: None,
        }
    }

    pub fn settings(&self) -> &PolicySettings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: PolicySettings) {
        let mut settings = settings;
        settings.thresholds = normalise_thresholds(&settings.thresholds);
        let previous = std::mem::replace(&mut self.settings, settings);
        if previous.thresholds != self.settings.thresholds {
            // Only a change to the threshold list re-arms anything: a threshold that is new
            // (or newly different) can fire, one the user already saw reported cannot. This
            // is what keeps "switch notifications on" from replaying old crossings.
            for state in self.devices.values_mut() {
                state.armed.retain(|t| self.settings.thresholds.contains(t));
                for threshold in &self.settings.thresholds {
                    if !state.armed.contains(threshold) {
                        state.armed.push(*threshold);
                    }
                }
            }
        }
    }

    /// Feed one reading and collect whatever is worth reporting.
    pub fn observe(
        &mut self,
        key: &str,
        name: &str,
        reading: &BatteryReading,
        now: Instant,
    ) -> Vec<Alert> {
        let settings = self.settings.clone();
        let state = self.devices.entry(key.to_string()).or_default();
        if !state.initialised {
            state.initialised = true;
            state.armed = settings.thresholds.clone();
        }

        // A charging device is not a low-battery story, and a failed battery is a different
        // story that the window shows but a notification would only get wrong.
        if reading.status.charging() {
            state.charging = true;
            state.armed = settings.thresholds.clone();
            // "Charged" is worth exactly one message per charge cycle. Devices disagree
            // about how they announce it — some go to FULL, others just sit at 100% while
            // still reporting RECHARGING — so both count.
            let full = matches!(
                reading.status,
                ChargeStatus::Full | ChargeStatus::AlmostFull
            ) || reading.percent.is_some_and(|p| p.value >= 95);
            if full && !state.charged_sent && settings.enabled {
                state.charged_sent = true;
                let percent = reading.percent.map(|p| p.value).unwrap_or(100);
                return vec![Alert::Charged {
                    key: key.to_string(),
                    name: name.to_string(),
                    percent,
                }];
            }
            return Vec::new();
        }

        if reading.status.faulty() {
            return Vec::new();
        }

        if state.charging {
            // Back on battery: this is a new discharge cycle.
            state.charging = false;
            state.charged_sent = false;
            state.armed = settings.thresholds.clone();
        }

        let Some(percent) = reading.percent.map(|p| p.value) else {
            return Vec::new();
        };

        let mut alerts = Vec::new();
        // Re-arm thresholds the device has recovered past.
        for threshold in settings.thresholds.iter().copied() {
            let recovered = percent >= threshold.saturating_add(settings.hysteresis);
            if recovered && !state.armed.contains(&threshold) {
                state.armed.push(threshold);
            }
        }

        // The highest armed threshold at or below the current level is the one to report.
        let hit = state.armed.iter().copied().filter(|t| percent <= *t).max();
        let Some(threshold) = hit else { return alerts };

        if !settings.enabled {
            // Notifications are off. Remember that these crossings were seen — every
            // threshold at or above the current level — so turning notifications on later
            // does not replay a backlog of levels the user chose to ignore.
            state.armed.retain(|t| *t < percent);
            return alerts;
        }
        if state.snoozed_until.is_some_and(|until| now < until) {
            // Snoozed: leave the threshold armed so it can speak once the snooze expires.
            return alerts;
        }

        // Global rate limit: a multi-device receiver must not produce a burst. A throttled
        // crossing stays armed and is reported on a later observation.
        if let Some(last) = self.last_alert
            && now.duration_since(last) < settings.rate_limit
        {
            return alerts;
        }

        state.armed.retain(|t| *t != threshold);
        self.last_alert = Some(now);
        alerts.push(Alert::Low {
            key: key.to_string(),
            name: name.to_string(),
            threshold,
            percent,
            estimated: reading.percent.is_some_and(|p| p.is_estimate()),
        });
        alerts
    }

    /// The user acknowledged this device: keep quiet for a while.
    pub fn snooze(&mut self, key: &str, now: Instant) {
        let until = now + self.settings.snooze;
        self.devices
            .entry(key.to_string())
            .or_default()
            .snoozed_until = Some(until);
    }

    /// The device went away or was re-paired: forget everything about it.
    pub fn reset(&mut self, key: &str) {
        self.devices.remove(key);
    }

    pub fn forget_everything(&mut self) {
        self.devices.clear();
        self.last_alert = None;
    }

    pub fn is_snoozed(&self, key: &str, now: Instant) -> bool {
        self.devices
            .get(key)
            .and_then(|state| state.snoozed_until)
            .is_some_and(|until| now < until)
    }

    /// Every device the policy currently remembers, so a caller can drop the ones that
    /// have gone away.
    pub fn known_keys(&self) -> Vec<String> {
        self.devices.keys().cloned().collect()
    }
}

fn normalise_thresholds(thresholds: &[u8]) -> Vec<u8> {
    let mut levels: Vec<u8> = thresholds
        .iter()
        .copied()
        .filter(|t| (1..=99).contains(t))
        .collect();
    levels.sort_unstable();
    levels.dedup();
    levels.reverse();
    if levels.is_empty() {
        levels = PolicySettings::default().thresholds;
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;
    use embr_hidpp::{BatteryReading, ChargeStatus, Percent, PercentSource};

    fn discharging(percent: u8) -> BatteryReading {
        BatteryReading::new(
            Some(Percent {
                value: percent,
                source: PercentSource::Estimated,
            }),
            None,
            ChargeStatus::Discharging,
        )
    }

    fn charging(percent: u8) -> BatteryReading {
        BatteryReading::new(
            Some(Percent {
                value: percent,
                source: PercentSource::Device,
            }),
            None,
            ChargeStatus::Recharging,
        )
    }

    fn policy() -> BatteryPolicy {
        BatteryPolicy::new(PolicySettings {
            enabled: true,
            ..PolicySettings::default()
        })
    }

    #[test]
    fn a_device_already_below_a_threshold_is_reported_on_first_sight() {
        // Starting the app with a flat mouse must not hide the problem until the *next*
        // threshold is crossed.
        let mut policy = policy();
        let alerts = policy.observe("k", "Mouse", &discharging(19), Instant::now());
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0], Alert::Low { threshold: 20, .. }));
    }

    #[test]
    fn crossing_fires_exactly_once() {
        let mut policy = policy();
        let t0 = Instant::now();
        let alerts = policy.observe("k", "Mouse", &discharging(21), t0);
        assert!(alerts.is_empty(), "above the threshold: nothing to say");
        let alerts = policy.observe(
            "k",
            "Mouse",
            &discharging(20),
            t0 + Duration::from_secs(120),
        );
        assert_eq!(alerts.len(), 1, "20% must fire once");
        assert_eq!(alerts[0].key(), "k");
        let alerts = policy.observe(
            "k",
            "Mouse",
            &discharging(19),
            t0 + Duration::from_secs(180),
        );
        assert!(alerts.is_empty(), "the same threshold must not fire twice");
        let alerts = policy.observe(
            "k",
            "Mouse",
            &discharging(10),
            t0 + Duration::from_secs(240),
        );
        assert_eq!(alerts.len(), 1, "the next threshold down is still armed");
        assert!(matches!(alerts[0], Alert::Low { threshold: 10, .. }));
    }

    #[test]
    fn hysteresis_re_arms_only_after_a_real_recovery() {
        let mut policy = policy();
        let t0 = Instant::now();
        assert_eq!(policy.observe("k", "Mouse", &discharging(20), t0).len(), 1);
        // 22 is above 20 but not past 20 + margin: still nothing
        assert!(
            policy
                .observe(
                    "k",
                    "Mouse",
                    &discharging(22),
                    t0 + Duration::from_secs(120)
                )
                .is_empty()
        );
        assert!(
            policy
                .observe(
                    "k",
                    "Mouse",
                    &discharging(20),
                    t0 + Duration::from_secs(180)
                )
                .is_empty()
        );
        // 26 is past 20 + 5: armed again, so the next crossing speaks
        assert!(
            policy
                .observe(
                    "k",
                    "Mouse",
                    &discharging(26),
                    t0 + Duration::from_secs(240)
                )
                .is_empty()
        );
        assert_eq!(
            policy
                .observe(
                    "k",
                    "Mouse",
                    &discharging(20),
                    t0 + Duration::from_secs(300)
                )
                .len(),
            1
        );
    }

    #[test]
    fn charging_suppresses_low_alerts_and_announces_one_full_charge() {
        let mut policy = policy();
        let t0 = Instant::now();
        assert!(
            policy.observe("k", "Mouse", &charging(15), t0).is_empty(),
            "plugged in at 15%: no low-battery nag"
        );
        let alerts = policy.observe("k", "Mouse", &charging(100), t0 + Duration::from_secs(100));
        assert!(matches!(alerts.as_slice(), [Alert::Charged { .. }]));
        assert!(
            policy
                .observe("k", "Mouse", &charging(100), t0 + Duration::from_secs(200))
                .is_empty(),
            "the charged message is once per cycle"
        );
    }

    #[test]
    fn a_faulty_battery_is_never_a_low_battery() {
        let mut policy = policy();
        let t0 = Instant::now();
        let reading = BatteryReading::new(
            Some(Percent {
                value: 3,
                source: PercentSource::Device,
            }),
            None,
            ChargeStatus::InvalidBattery,
        );
        assert!(policy.observe("k", "Mouse", &reading, t0).is_empty());
    }

    #[test]
    fn the_rate_limit_holds_across_devices() {
        let mut policy = policy();
        let t0 = Instant::now();
        assert_eq!(policy.observe("a", "Mouse", &discharging(20), t0).len(), 1);
        assert!(
            policy
                .observe(
                    "b",
                    "Keyboard",
                    &discharging(20),
                    t0 + Duration::from_secs(5)
                )
                .is_empty(),
            "a second device within the rate limit stays quiet"
        );
        assert_eq!(
            policy
                .observe(
                    "b",
                    "Keyboard",
                    &discharging(20),
                    t0 + Duration::from_secs(61)
                )
                .len(),
            1,
            "…and speaks once the window has passed"
        );
    }

    #[test]
    fn snoozing_silences_one_device_only() {
        let mut policy = policy();
        let t0 = Instant::now();
        policy.snooze("a", t0);
        assert!(policy.is_snoozed("a", t0 + Duration::from_secs(60)));
        assert!(
            policy
                .observe("a", "Mouse", &discharging(20), t0 + Duration::from_secs(60))
                .is_empty()
        );
        assert_eq!(
            policy
                .observe(
                    "b",
                    "Keyboard",
                    &discharging(20),
                    t0 + Duration::from_secs(60)
                )
                .len(),
            1
        );
        assert_eq!(
            policy
                .observe(
                    "a",
                    "Mouse",
                    &discharging(20),
                    t0 + Duration::from_secs(31 * 60)
                )
                .len(),
            1,
            "after the snooze expires the device speaks again"
        );
    }

    #[test]
    fn a_disabled_policy_never_speaks_but_still_tracks() {
        let mut policy = BatteryPolicy::new(PolicySettings {
            enabled: false,
            ..PolicySettings::default()
        });
        let t0 = Instant::now();
        assert!(policy.observe("k", "Mouse", &discharging(5), t0).is_empty());
        // switching it on later must not fire for a level the user chose to ignore
        policy.set_settings(PolicySettings {
            enabled: true,
            ..PolicySettings::default()
        });
        assert!(
            policy
                .observe("k", "Mouse", &discharging(5), t0 + Duration::from_secs(1))
                .is_empty()
        );
    }

    #[test]
    fn reset_forgets_a_device() {
        let mut policy = policy();
        let t0 = Instant::now();
        assert_eq!(policy.observe("k", "Mouse", &discharging(20), t0).len(), 1);
        policy.reset("k");
        assert_eq!(
            policy
                .observe(
                    "k",
                    "Mouse",
                    &discharging(20),
                    t0 + Duration::from_secs(120)
                )
                .len(),
            1,
            "a re-paired device is a new device"
        );
    }

    #[test]
    fn a_reading_without_a_percentage_is_ignored() {
        let mut policy = policy();
        let reading = BatteryReading::new(None, Some(3500), ChargeStatus::Discharging);
        assert!(
            policy
                .observe("k", "Mouse", &reading, Instant::now())
                .is_empty()
        );
    }

    #[test]
    fn alerts_carry_usable_text() {
        let alert = Alert::Low {
            key: "k".into(),
            name: "G703 LIGHTSPEED".into(),
            threshold: 20,
            percent: 18,
            estimated: true,
        };
        let (title, body) = alert.title_and_body();
        assert_eq!(title, "Battery low");
        assert!(body.contains("about 18%"), "{body}");
        assert!(body.contains("below 20%"), "{body}");
    }

    #[test]
    fn thresholds_are_normalised_on_construction() {
        let policy = BatteryPolicy::new(PolicySettings {
            thresholds: vec![0, 10, 20, 20, 200],
            ..PolicySettings::default()
        });
        assert_eq!(policy.settings().thresholds, vec![20, 10]);
    }
}
