//! The vocabulary the three threads use to talk to each other.
//!
//! There is exactly one device thread, one UI thread and one notification thread. They never
//! share a device handle, a `Monitor` or a Slint object: they pass these messages instead.
//! That is what keeps a five-second HID++ sweep from touching the event loop, and a slow
//! D-Bus server from touching either.

use std::sync::mpsc::Sender;

use embr_core::{Alert, Snapshot, TrayLook, TrayStyle, TrayTextColor};

/// UI thread or tray → the device thread.
#[derive(Debug, Clone)]
pub enum Command {
    /// Read every device now, on the application's own schedule: quick, and a sleeping device
    /// is not worth stalling for.
    Refresh,
    /// Read every device now because the user asked, with the longer waits that give a device
    /// coming back from sleep a chance to answer.
    Rescan,
    /// Ask every device whether it is still there, with deadlines short enough to do it every few
    /// seconds. This is what notices a device switching off while the user is looking at the
    /// window, instead of the next scheduled sweep finding out.
    Watch,
    /// Keep quiet about one device for the configured snooze period.
    Snooze(String),
    /// Turn battery notifications on or off and remember the choice.
    SetNotifications(bool),
    /// Give one device its own alert level, or (`None`) put it back on the shared thresholds.
    SetDeviceAlertLevel { key: String, level: Option<u8> },
    /// Draw the tray icon in another style.
    SetTrayStyle(TrayStyle),
    /// Draw the percentage style's digits in another colour.
    SetTrayTextColor(TrayTextColor),
    /// Give one device a tray icon of its own, or take it away again.
    SetTrayDevice { key: String, on: bool },
    /// Keep running when the window is closed (tray only), or quit instead.
    SetCloseToTray(bool),
    /// Ask the devices whether they are still there, while the window is open, or leave it to the
    /// sweeps.
    SetWatchDevices(bool),
    /// How often a fresh reading is taken while the window is open.
    SetRefreshSeconds(u64),
}

/// The device thread, the notification thread or the tray → the UI thread.
///
/// The UI thread polls these on its own timer instead of being called into from another
/// thread. Tray clicks therefore arrive within half a second, and no Slint object ever
/// crosses a thread boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiRequest {
    ShowWindow,
    /// Show the window on one particular device — what clicking an icon that is about a device
    /// means.
    ShowDevice(String),
    Quit,
}

/// Everything the window needs to draw one refresh, plus the settings that came with it.
///
/// The device list and the ages are `core`'s own view model and text — the UI only unpacks
/// them. The settings fields are configuration *presentation*: `core` stores thresholds as a
/// list of numbers and this is the list as a sentence.
#[derive(Debug, Clone)]
pub struct UiState {
    pub snapshot: Snapshot,
    pub notifications_enabled: bool,
    pub thresholds_text: String,
    pub refresh_seconds: u64,
    /// Which shape the tray icon is drawn as, and the colour of its text when it has any.
    pub tray_look: TrayLook,
    /// How long a snooze keeps quiet, so the row beside the device can say so.
    pub snooze_minutes: u64,
    /// The devices the user gave a tray icon of their own, by key.
    pub tray_devices: Vec<String>,
    pub close_to_tray: bool,
    /// Whether a device switching off is noticed within seconds rather than at the next sweep.
    pub watch_devices: bool,
    /// Where the settings on the settings page are written, so the page can say so.
    pub config_path: String,
    pub notice: Option<String>,
}

/// Something for the notification thread to put on screen.
///
/// Battery alerts and the occasional word from the application itself travel the same channel:
/// both are "say this to the user", and keeping them one channel keeps one thread owning D-Bus.
#[derive(Debug, Clone)]
pub enum Notice {
    /// A battery alert, decided by the policy in `core`.
    Alert(Alert),
    /// A line from the app: no icon of its own, no action, just a sentence.
    Text { title: String, body: String },
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

/// `quiet about this device for 30 minutes` — the caption under the Snooze row, from the
/// configured period rather than a number written into the window.
pub fn snooze_caption(minutes: u64) -> String {
    match minutes {
        0 => "quiet about this device until you say otherwise".to_string(),
        1 => "quiet about this device for a minute".to_string(),
        minutes => format!("quiet about this device for {minutes} minutes"),
    }
}

/// Hand an alert to the notification thread, or say why it could not be handed over.
pub fn dispatch_alert(notifications: &Sender<Notice>, alert: Alert) {
    let (title, body) = alert.title_and_body();
    if notifications.send(Notice::Alert(alert)).is_err() {
        eprintln!("[notify] the notification thread has stopped; dropped: {title}: {body}");
    }
}

/// Hand the notification thread a line of the app's own — no action, nothing to click.
pub fn dispatch_note(notifications: &Sender<Notice>, title: &str, body: &str) {
    let notice = Notice::Text {
        title: title.to_string(),
        body: body.to_string(),
    };
    if notifications.send(notice).is_err() {
        eprintln!("[notify] the notification thread has stopped; dropped: {title}: {body}");
    }
}
