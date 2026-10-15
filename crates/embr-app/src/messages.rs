//! The vocabulary the three threads use to talk to each other.
//!
//! There is exactly one device thread, one UI thread and one notification thread. They never
//! share a device handle, a `Monitor` or a Slint object: they pass these messages instead.
//! That is what keeps a five-second HID++ sweep from touching the event loop, and a slow
//! D-Bus server from touching either.

use std::sync::mpsc::Sender;

use embr_core::{Alert, Snapshot};

/// UI thread or tray → the device thread.
#[derive(Debug, Clone)]
pub enum Command {
    /// Read every device now.
    Refresh,
    /// Keep quiet about one device for the configured snooze period.
    Snooze(String),
    /// Turn battery notifications on or off and remember the choice.
    SetNotifications(bool),
}

/// The device thread, the notification thread or the tray → the UI thread.
///
/// The UI thread polls these on its own timer instead of being called into from another
/// thread. Tray clicks therefore arrive within half a second, and no Slint object ever
/// crosses a thread boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiRequest {
    ShowWindow,
    Quit,
}

/// Everything the window needs to draw one refresh, plus the settings that came with it.
///
/// The device list, the headline and the ages are `core`'s own view model and text — the UI
/// only unpacks them. The four settings fields are configuration *presentation*: `core`
/// stores thresholds as a list of numbers and this is the list as a sentence.
#[derive(Debug, Clone)]
pub struct UiState {
    pub snapshot: Snapshot,
    pub notifications_enabled: bool,
    pub thresholds_text: String,
    pub refresh_seconds: u64,
    pub notice: Option<String>,
}

/// `20 % / 10 %` — the thresholds as the settings card shows them.
pub fn thresholds_text(thresholds: &[u8]) -> String {
    if thresholds.is_empty() {
        return "—".to_string();
    }
    thresholds
        .iter()
        .map(|level| format!("{level} %"))
        .collect::<Vec<String>>()
        .join(" / ")
}

/// Hand an alert to the notification thread, or say why it could not be handed over.
pub fn dispatch_alert(alerts: &Sender<Alert>, alert: Alert) {
    let (title, body) = alert.title_and_body();
    if alerts.send(alert).is_err() {
        eprintln!("[notify] the notification thread has stopped; dropped: {title}: {body}");
    }
}
