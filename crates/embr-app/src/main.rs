//! Embr — the window that shows what the Logitech devices on this machine are doing.
//!
//! Three threads, and nothing shared between them:
//!
//! * the **UI thread** owns everything Slint: it draws, it collects commands and it polls the
//!   other two on a timer. It never opens a device and never talks to D-Bus.
//! * the **device thread** ([`worker`]) owns `core::Monitor` and does the HID++ work, which
//!   takes seconds;
//! * the **notification thread** ([`notify`]) owns D-Bus and can block all it likes.
//!
//! plus the tray ([`tray`]) on its own thread inside `ksni`.
//!
//! Cross-thread traffic is three channels — [`Command`] inward, [`UiState`] and
//! [`UiRequest`] outward — and never a Slint object.
//!
//! Everything the window shows comes from `core`: the device list, the percentage text with
//! its `~` for an estimate, the status label, the tray tooltip and the ages. This crate draws
//! them and nothing else.

mod messages;
mod notify;
mod tray;
mod view;
mod worker;

slint::include_modules!();

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Instant, SystemTime};

use embr_core::Alert;
use slint::{Model, ModelRc, VecModel};

use messages::{Command, UiRequest, UiState};

/// The last thing the device thread said, kept so the details card and the ages can be drawn
/// again between refreshes.
type SharedState = Rc<RefCell<Option<UiState>>>;
/// The selected device, remembered by its stable key: the list is re-sorted on every pass.
type Selection = Rc<RefCell<Option<String>>>;

/// The last value pushed into every control the user can also move.
///
/// A `Switch` fires when its value changes — including when we are the ones changing it — and
/// the alert level must not be dragged back to the old number while the user is still moving
/// it. "What is still in flight?" is what tells a user's action from our own echo.
#[derive(Clone)]
struct Pushed {
    notifications: Rc<Cell<bool>>,
    tray_percent: Rc<Cell<bool>>,
    /// An alert level the user set that the device thread has not confirmed yet; `-1` when
    /// there is nothing in flight.
    alert_pending: Rc<Cell<i32>>,
}

impl Pushed {
    fn new() -> Self {
        Self {
            notifications: Rc::new(Cell::new(false)),
            tray_percent: Rc::new(Cell::new(false)),
            alert_pending: Rc::new(Cell::new(-1)),
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let launched = Instant::now();
    let window = AppWindow::new()?;
    // Created up front and shown on demand: a Slint window is cheap when it is not visible,
    // and creating one from inside an event handler is a good way to find edge cases.
    let about = AboutWindow::new()?;

    let (commands, command_queue) = mpsc::channel::<Command>();
    let (states, state_queue) = mpsc::channel::<UiState>();
    let (alerts, alert_queue) = mpsc::channel::<Alert>();
    let (requests, request_queue) = mpsc::channel::<UiRequest>();

    // --- the tray: registered normally, drawn only if Noctalia has it pinned (see README) ---
    let tray_service =
        ksni::TrayService::new(tray::EmbrTray::new(commands.clone(), requests.clone()));
    let tray_handle = tray_service.handle();
    tray_service.spawn();
    eprintln!(
        "[app] StatusNotifierItem registered with id \"embr\" (Noctalia draws it only when pinned)"
    );

    // --- the threads that are allowed to be slow ---
    match notify::spawn(alert_queue, requests.clone()) {
        Ok(_) => {}
        Err(problem) => eprintln!("[app] no notification thread: {problem}"),
    }
    match worker::spawn(command_queue, states, alerts, Some(tray_handle)) {
        Ok(_) => {}
        Err(problem) => eprintln!("[app] no device thread: {problem} — the window will stay empty"),
    }

    let rows: Rc<VecModel<Device>> = Rc::new(VecModel::default());
    window.set_devices(ModelRc::from(rows.clone()));
    window.set_refreshing(true);

    let latest: SharedState = Rc::new(RefCell::new(None));
    let selection: Selection = Rc::new(RefCell::new(None));
    let pushed = Pushed::new();

    window.on_request_refresh({
        let window = window.as_weak();
        let commands = commands.clone();
        move || {
            if let Some(window) = window.upgrade() {
                window.set_refreshing(true);
            }
            if commands.send(Command::Refresh).is_err() {
                eprintln!("[app] the device thread has stopped; cannot refresh");
            }
        }
    });

    window.on_pick({
        let window = window.as_weak();
        let latest = latest.clone();
        let selection = selection.clone();
        move |index| {
            if let Some(state) = latest.borrow().as_ref()
                && let Some(device) = state.snapshot.devices.get(index.max(0) as usize)
            {
                *selection.borrow_mut() = Some(device.key.clone());
            }
            if let Some(window) = window.upgrade() {
                show_selected(&window, &latest, &selection);
            }
        }
    });

    window.on_snooze({
        let commands = commands.clone();
        move |key| {
            let key = key.to_string();
            eprintln!("[app] snooze requested for {key}");
            if commands.send(Command::Snooze(key)).is_err() {
                eprintln!("[app] the device thread has stopped; nothing was snoozed");
            }
        }
    });

    window.on_notifications_toggled({
        let commands = commands.clone();
        let pushed = pushed.clone();
        move |enabled| {
            if enabled == pushed.notifications.get() {
                // The value we pushed into the switch coming back at us, not a user action.
                return;
            }
            pushed.notifications.set(enabled);
            eprintln!(
                "[app] battery notifications switched {}",
                if enabled { "on" } else { "off" }
            );
            if commands.send(Command::SetNotifications(enabled)).is_err() {
                eprintln!("[app] the device thread has stopped; the choice was not saved");
            }
        }
    });

    // The spin box shows the selected device's alert level. A user's move is remembered where
    // the switch remembers too, so the next snapshot can tell an edit in flight from a value
    // that simply changed.
    window.on_alert_level_changed({
        let commands = commands.clone();
        let latest = latest.clone();
        let selection = selection.clone();
        let pushed = pushed.clone();
        move |level| {
            let level = level.clamp(1, 99) as u8;
            pushed.alert_pending.set(i32::from(level));
            let Some(key) = selected_key(&latest, &selection) else {
                eprintln!("[app] no device is selected; the alert level was not saved");
                return;
            };
            eprintln!("[app] {key} alerts at {level}% from now on");
            let command = Command::SetDeviceAlertLevel {
                key,
                level: Some(level),
            };
            if commands.send(command).is_err() {
                eprintln!("[app] the device thread has stopped; the level was not saved");
            }
        }
    });

    window.on_alert_level_cleared({
        let commands = commands.clone();
        let latest = latest.clone();
        let selection = selection.clone();
        let pushed = pushed.clone();
        move || {
            // What the shared thresholds will read is not known here until the device thread
            // answers, so the box is left alone until that snapshot arrives.
            pushed.alert_pending.set(-1);
            let Some(key) = selected_key(&latest, &selection) else {
                eprintln!("[app] no device is selected; there was nothing to clear");
                return;
            };
            eprintln!("[app] {key} goes back to the shared thresholds");
            let command = Command::SetDeviceAlertLevel { key, level: None };
            if commands.send(command).is_err() {
                eprintln!("[app] the device thread has stopped; the level was not cleared");
            }
        }
    });

    window.on_tray_percent_toggled({
        let commands = commands.clone();
        let pushed = pushed.clone();
        move |enabled| {
            if enabled == pushed.tray_percent.get() {
                return;
            }
            pushed.tray_percent.set(enabled);
            eprintln!(
                "[app] the tray icon shows {}",
                if enabled {
                    "the lowest percentage"
                } else {
                    "a battery"
                }
            );
            if commands.send(Command::SetTrayPercent(enabled)).is_err() {
                eprintln!("[app] the device thread has stopped; the choice was not saved");
            }
        }
    });

    window.on_show_about({
        let about = about.as_weak();
        move || {
            if let Some(about) = about.upgrade()
                && let Err(problem) = about.show()
            {
                eprintln!("[app] could not show the About window: {problem}");
            }
        }
    });

    // The About window is otherwise reachable only by clicking, which makes it awkward to look at
    // when checking a change to it. Same shape as `EMBR_DEBUG_ALERT` in the worker — and it is
    // raised from inside the event loop, because showing it beforehand leaves it behind the main
    // window.
    if std::env::var_os("EMBR_DEBUG_ABOUT").is_some() {
        let about = about.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(500), move || {
            if let Some(about) = about.upgrade()
                && let Err(problem) = about.show()
            {
                eprintln!("[app] EMBR_DEBUG_ABOUT: could not show the About window: {problem}");
            }
        });
    }

    about.on_close_requested({
        let about = about.as_weak();
        move || {
            if let Some(about) = about.upgrade()
                && let Err(problem) = about.hide()
            {
                eprintln!("[app] could not hide the About window: {problem}");
            }
        }
    });

    window.on_tick({
        let window = window.as_weak();
        let latest = latest.clone();
        let selection = selection.clone();
        let rows = rows.clone();
        let pushed = pushed.clone();
        move || {
            // 1. What the tray or a notification asked the window to do.
            while let Ok(request) = request_queue.try_recv() {
                match request {
                    UiRequest::ShowWindow => {
                        if let Some(window) = window.upgrade() {
                            if let Err(problem) = window.show() {
                                eprintln!("[app] could not show the window: {problem}");
                            }
                            window.window().set_minimized(false);
                        }
                    }
                    UiRequest::Quit => {
                        eprintln!("[app] quit requested");
                        if let Err(problem) = slint::quit_event_loop() {
                            eprintln!("[app] could not stop the event loop: {problem}");
                        }
                    }
                }
            }
            // 2. What the device thread has produced since the last tick.
            while let Ok(state) = state_queue.try_recv() {
                if let Some(window) = window.upgrade() {
                    apply(&window, &latest, &selection, &rows, &pushed, state);
                }
            }
            // 3. Keep the relative ages honest.
            if let Some(window) = window.upgrade() {
                tick_ages(&window, &latest, &selection, &rows);
            }
        }
    });

    window.show()?;
    eprintln!(
        "[app] window shown {} ms after launch",
        launched.elapsed().as_millis()
    );
    if let Err(problem) = slint::run_event_loop() {
        eprintln!("[app] the event loop ended with {problem}");
    }
    eprintln!("[app] the event loop has returned; exiting");
    Ok(())
}

/// Take one snapshot into the window: the list, the detail card and the settings.
fn apply(
    window: &AppWindow,
    latest: &SharedState,
    selection: &Selection,
    rows: &Rc<VecModel<Device>>,
    pushed: &Pushed,
    state: UiState,
) {
    // The selection survives a refresh, and only a device that is still there can stay
    // selected; otherwise the first (worst) device is shown.
    let wanted = selection.borrow().clone();
    let keep = wanted.filter(|key| {
        state
            .snapshot
            .devices
            .iter()
            .any(|device| device.key == *key)
    });
    *selection.borrow_mut() = keep.or_else(|| {
        state
            .snapshot
            .devices
            .first()
            .map(|device| device.key.clone())
    });

    rows.set_vec(
        state
            .snapshot
            .devices
            .iter()
            .map(view::device)
            .collect::<Vec<_>>(),
    );

    window.set_refresh_seconds(state.refresh_seconds as i32);
    window.set_thresholds_text(state.thresholds_text.clone().into());
    window.set_notice(state.notice.clone().unwrap_or_default().into());
    pushed.notifications.set(state.notifications_enabled);
    window.set_notifications_enabled(state.notifications_enabled);
    pushed.tray_percent.set(state.tray_shows_percent);
    window.set_tray_percent(state.tray_shows_percent);
    apply_alert_level(window, &state, selection, pushed);
    window.set_refreshing(false);

    *latest.borrow_mut() = Some(state);
    tick_ages(window, latest, selection, rows);
}

/// Put the selected device's alert level into the spin box — unless a snapshot from before
/// the user's edit is what is in hand, in which case it is the older number and has no
/// business overwriting the newer one. The snapshot that confirms the edit is what closes
/// that window, and it also drags the box in line if the two ever disagree.
fn apply_alert_level(window: &AppWindow, state: &UiState, selection: &Selection, pushed: &Pushed) {
    let selected = selection.borrow().clone();
    let (level, custom) = state
        .snapshot
        .devices
        .iter()
        .find(|device| Some(&device.key) == selected.as_ref())
        .map(|device| {
            (
                i32::from(device.alert_below_effective),
                device.alert_below.is_some(),
            )
        })
        .unwrap_or((20, false));

    let pending = pushed.alert_pending.get();
    if pending == level {
        // The device thread has agreed with what the user set: nothing is in flight any more.
        pushed.alert_pending.set(-1);
    } else if pending >= 0 {
        return;
    }

    if window.get_alert_level() != level {
        window.set_alert_level(level);
    }
    window.set_alert_custom(custom);
}

/// Draw the selected device from the latest snapshot. Safe to call at any time: without a
/// snapshot, or without a device, the card says so instead of showing a stale device.
fn show_selected(window: &AppWindow, latest: &SharedState, selection: &Selection) {
    let borrowed = latest.borrow();
    let Some(state) = borrowed.as_ref() else {
        return;
    };
    let devices = &state.snapshot.devices;
    window.set_has_devices(!devices.is_empty());

    let selected = selection.borrow().clone();
    let index = selected
        .and_then(|key| devices.iter().position(|device| device.key == key))
        .unwrap_or(0);
    match devices.get(index) {
        Some(view) => {
            window.set_selected_index(index as i32);
            window.set_selected(view::device(view));
        }
        None => {
            window.set_selected_index(-1);
            window.set_selected(view::nothing());
        }
    }
}

/// The two ages the window shows — the snapshot's and the selected device's — plus the rows
/// whose own age has moved on a second.
fn tick_ages(
    window: &AppWindow,
    latest: &SharedState,
    selection: &Selection,
    rows: &Rc<VecModel<Device>>,
) {
    let borrowed = latest.borrow();
    let Some(state) = borrowed.as_ref() else {
        return;
    };

    match state.snapshot.taken {
        Some(taken) => {
            let age = embr_core::human_age(taken, SystemTime::now());
            window.set_last_updated_label(age.into());
        }
        None => window.set_last_updated_label("never".into()),
    }

    // A row is only rewritten when its age text has actually changed, so the list is not
    // rebuilt twice a second for nothing.
    for (index, device) in state.snapshot.devices.iter().enumerate() {
        let age = view::age(device);
        let stale = rows
            .row_data(index)
            .is_some_and(|row| row.updated.as_str() != age.as_str());
        if stale {
            rows.set_row_data(index, view::device(device));
        }
    }

    show_selected(window, latest, selection);
}

/// The key of the device the details card is showing, if there is one — which device a setting
/// phrased as "this device" applies to.
fn selected_key(latest: &SharedState, selection: &Selection) -> Option<String> {
    let key = selection.borrow().clone()?;
    let borrowed = latest.borrow();
    let state = borrowed.as_ref()?;
    state
        .snapshot
        .devices
        .iter()
        .any(|device| device.key == key)
        .then_some(key)
}
