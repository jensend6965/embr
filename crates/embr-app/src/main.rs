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
//! its `~` for an estimate, the status label, the headline, the tray tooltip and the ages.
//! This crate draws them and nothing else.

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
    // What the switch was last told to be: used to tell a user's click from our own update.
    let switch_value = Rc::new(Cell::new(false));

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
        let switch_value = switch_value.clone();
        move |enabled| {
            if enabled == switch_value.get() {
                // The value we pushed into the switch coming back at us, not a user action.
                return;
            }
            switch_value.set(enabled);
            eprintln!(
                "[app] battery notifications switched {}",
                if enabled { "on" } else { "off" }
            );
            if commands.send(Command::SetNotifications(enabled)).is_err() {
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
        let switch_value = switch_value.clone();
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
                    apply(&window, &latest, &selection, &rows, &switch_value, state);
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

/// Take one snapshot into the window: the list, the headline and the settings.
fn apply(
    window: &AppWindow,
    latest: &SharedState,
    selection: &Selection,
    rows: &Rc<VecModel<Device>>,
    switch_value: &Rc<Cell<bool>>,
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

    window.set_headline(state.snapshot.headline().into());
    window.set_refresh_seconds(state.refresh_seconds as i32);
    window.set_thresholds_text(state.thresholds_text.clone().into());
    window.set_notice(state.notice.clone().unwrap_or_default().into());
    switch_value.set(state.notifications_enabled);
    window.set_notifications_enabled(state.notifications_enabled);
    window.set_refreshing(false);

    *latest.borrow_mut() = Some(state);
    tick_ages(window, latest, selection, rows);
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
            window.set_last_updated(format!("Last updated {age}").into());
            window.set_last_updated_label(age.into());
        }
        None => {
            window.set_last_updated("Not updated yet".into());
            window.set_last_updated_label("never".into());
        }
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
