//! The vocabulary the window, the settings page and the device thread use to talk to each other.
//!
//! The device thread owns `core::Monitor` and never shares it, the UI threads own every widget and
//! never open a device, and the two pass these messages instead. A sweep is seconds of HID++
//! traffic; it must not run where the pixels are drawn.

use std::sync::mpsc::Sender;

use embr_core::{Snapshot, Theme, TrayLook, TrayStyle, TrayTextColor};

/// The window (or the settings page) → the device thread.
#[derive(Debug, Clone)]
pub enum Command {
    /// Read every device now, on the application's own schedule: quick, and a sleeping device is
    /// not worth stalling for.
    Refresh,
    /// Read every device now because the user asked, with the longer waits that give a device
    /// coming back from sleep a chance to answer.
    Rescan,
    /// Ask every device whether it is still there, with deadlines short enough to do it every few
    /// seconds.
    Watch,
    /// Ask one device which features it implements, because the window is about to show what it
    /// can be asked about. A device's inventory, so it is asked once and kept.
    Pages(String),
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
    /// Ask the devices whether they are still there between sweeps, or leave it to the sweeps.
    SetWatchDevices(bool),
    /// Keep running when the window is closed (the tray only), or quit instead.
    SetCloseToTray(bool),
    /// How often a fresh reading is taken while the window is open.
    SetRefreshSeconds(u64),
    /// Which colour scheme the window should use.
    SetTheme(Theme),
    /// Whether the readouts carry the voltage next to the percentage.
    SetShowVoltage(bool),
    /// Whether a reading is drawn in the colour of its level.
    SetLevelColors(bool),
    /// Whether a percentage inferred from voltage is marked as approximate.
    SetMarkEstimates(bool),
}

/// The device thread → the window: everything one refresh produced, plus the settings that came
/// with it.
#[derive(Debug, Clone)]
pub struct State {
    pub snapshot: Snapshot,
    pub notifications_enabled: bool,
    /// The shared alert thresholds as a sentence, from core: the window draws words, it does not
    /// choose them.
    pub thresholds_text: String,
    /// The cadence the user configured, which the window's own clock turns into sweeps.
    pub refresh_seconds: u64,
    /// Which shape the tray icon is drawn as, and the colour of its text when it has any.
    pub tray_look: TrayLook,
    /// How long a snooze keeps quiet, so the settings page can say so.
    pub snooze_minutes: u64,
    /// The devices the user gave a tray icon of their own, by key.
    pub tray_devices: Vec<String>,
    pub close_to_tray: bool,
    /// Which colour scheme the window was asked to use.
    pub theme: Theme,
    /// Whether a reading's voltage is shown beside its percentage.
    pub show_voltage: bool,
    /// Whether a reading is drawn in the colour of its level.
    pub level_colors: bool,
    /// Whether a percentage inferred from voltage is marked as approximate.
    pub mark_estimates: bool,
    /// Whether a device switching off is noticed within seconds rather than at the next sweep.
    pub watch_devices: bool,
    /// Where the settings are written, so the page can say so.
    pub config_path: String,
    /// A line worth showing once — a config file that could not be read, a device lost on the way.
    pub notice: Option<String>,
}

/// What the user did in the window, delivered to the UI thread's own tick.
///
/// Signals report a click the way they arrive — through the widget that was clicked — rather than
/// reaching into the window from wherever they are connected, so the device the window shows
/// changes in one place.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// Open one device: expand its row and show it.
    Select(String),
    /// The tray picker moved: draw the icons now, the way the switch it owns does, rather than
    /// queueing the change behind whatever sweep is in flight.
    RestyleTray(TrayLook),
    /// Quit, from the settings page's button.
    Quit,
}

/// Hand the device thread a command, or say why it could not be handed over.
pub fn dispatch(commands: &Sender<Command>, command: Command) {
    if commands.send(command).is_err() {
        eprintln!("[app] the device thread has stopped; nothing was asked of the devices");
    }
}

/// The same, for an event the UI thread handles itself.
pub fn dispatch_event(events: &Sender<UiEvent>, event: UiEvent) {
    if events.send(event).is_err() {
        eprintln!("[app] the window is gone; the click was dropped");
    }
}
