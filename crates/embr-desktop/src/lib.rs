//! What Embr is on the desktop, with no opinion about the window toolkit: the tray icons and the
//! desktop notifications.
//!
//! They lived in the window crate until the window was ported from Slint to GTK, at which point
//! copying them would have meant two tray implementations. They never used a toolkit — the tray is
//! D-Bus (`ksni`) and the notifications are D-Bus (`notify-rust`), and the pictograms are drawn as
//! plain ARGB32 pixmaps — so they moved down a layer instead. One function decides what a tray icon
//! looks like: the live item and every picker's preview both call [`tray::glyph`].
//!
//! What is *not* here is anything a window does with the bytes: a window wants a texture, and the
//! one window in this repository does that conversion in its own adapter.

pub mod notify;
pub mod tray;

/// A tray pictogram: 22×22 pixels of ARGB in network byte order, which is what a
/// StatusNotifierItem carries and what GTK reads as `gdk::MemoryFormat::A8r8g8b8`. Named here so
/// that a window depends on this crate rather than on the D-Bus library underneath it.
pub use ksni::Icon as Pictogram;

use std::sync::mpsc::Sender;

use embr_core::Alert;

/// What a tray icon asks the application to do.
///
/// The seam between the tray and a window: a click is reported as one of these on a channel the
/// window already drains, so the tray holds no widget and no `Monitor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    /// A thorough sweep, because the user asked for one.
    Rescan,
    /// Show the window — on one device when the icon was about one.
    ShowWindow { key: Option<String> },
    /// Quit, from the tray's menu.
    Quit,
}

/// Something for the notification thread to put on screen.
///
/// Battery alerts and the occasional word from the application travel the same channel: both are
/// "say this to the user", and keeping them one channel keeps one thread owning D-Bus.
#[derive(Debug, Clone)]
pub enum Notice {
    /// A battery alert, decided by the policy in `core`.
    Alert(Alert),
    /// A line from the app: no icon of its own, no action, just a sentence.
    Text { title: String, body: String },
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
