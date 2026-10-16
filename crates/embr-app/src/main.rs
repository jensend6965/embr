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
use std::time::{Duration, Instant, SystemTime};

use embr_core::{TrayLook, TrayStyle, TrayTextColor, WindowClose, window_close_action};
use slint::{CloseRequestResponse, Model, ModelRc, VecModel};

use messages::{Command, Notice, UiRequest, UiState, dispatch_note};

/// The last thing the device thread said, kept so the details card and the ages can be drawn
/// again between refreshes.
type SharedState = Rc<RefCell<Option<UiState>>>;
/// The selected device, remembered by its stable key: the list is re-sorted on every pass.
type Selection = Rc<RefCell<Option<String>>>;

/// The shortest gap between a sweep and another one forced by a device arriving or leaving. A
/// dongle announces itself as several devices within a few milliseconds and the socket is drained
/// in one call, so that is a single sweep; this is for the device stuck re-enumerating, which
/// must not get a sweep per attempt.
const SETTLE: Duration = Duration::from_secs(2);

/// The last value pushed into every control the user can also move.
///
/// A `Switch` fires when its value changes — including when we are the ones changing it — and a
/// number the user is moving must not be dragged back to the old one by a snapshot taken before
/// their edit. "What is still in flight?" is what tells a user's action from our own echo.
#[derive(Clone)]
struct Pushed {
    notifications: Rc<Cell<bool>>,
    tray_look: Rc<Cell<TrayLook>>,
    close_to_tray: Rc<Cell<bool>>,
    watch_devices: Rc<Cell<bool>>,
    alert_pending: Pending,
    refresh_pending: Pending,
}

impl Pushed {
    fn new() -> Self {
        Self {
            notifications: Rc::new(Cell::new(false)),
            tray_look: Rc::new(Cell::new(TrayLook::default())),
            close_to_tray: Rc::new(Cell::new(true)),
            watch_devices: Rc::new(Cell::new(true)),
            alert_pending: Pending::new(),
            refresh_pending: Pending::new(),
        }
    }
}

/// The three models the window draws from, kept together because every snapshot writes all of
/// them and a function that takes them one by one stops being readable.
#[derive(Clone)]
struct Models {
    devices: Rc<VecModel<Device>>,
    tray_choices: Rc<VecModel<TrayChoice>>,
    tray_styles: Rc<VecModel<TrayStyleOption>>,
    tray_colors: Rc<VecModel<TrayColorOption>>,
}

impl Models {
    fn new() -> Self {
        Self {
            devices: Rc::new(VecModel::default()),
            tray_choices: Rc::new(VecModel::default()),
            tray_styles: Rc::new(VecModel::default()),
            tray_colors: Rc::new(VecModel::default()),
        }
    }

    /// Show one look as the chosen one: both pickers, the sentence under the first, and the record
    /// of what the window has already drawn. Called when the user picks something and when a
    /// snapshot reports a look the window has not drawn yet (a fresh start, or a config edited by
    /// hand).
    fn show_tray_look(&self, window: &AppWindow, pushed: &Pushed, look: TrayLook) {
        self.tray_styles.set_vec(
            TrayStyle::ALL
                .into_iter()
                .map(|candidate| view::tray_style_option(candidate, look))
                .collect::<Vec<_>>(),
        );
        self.tray_colors.set_vec(
            TrayTextColor::ALL
                .into_iter()
                .map(|candidate| view::tray_color_option(candidate, look.text_color))
                .collect::<Vec<_>>(),
        );
        window.set_tray_style_caption(view::tray_style_caption(look.style).into());
        // The one style that has any text is the percentage: nothing else reads a colour.
        window.set_tray_has_text(look.style == TrayStyle::Percent);
        pushed.tray_look.set(look);
    }
}

/// A number the user moved, that the device thread has not confirmed yet.
#[derive(Clone)]
struct Pending(Rc<Cell<i32>>);

impl Pending {
    fn new() -> Self {
        Self(Rc::new(Cell::new(-1)))
    }

    /// Remember what the user just set.
    fn set(&self, value: i32) {
        self.0.set(value);
    }

    /// Whether a value arriving from the device thread may be written into the widget: yes when
    /// it is the one the user set (the round trip is done, so it is confirmed and forgotten), and
    /// yes when nothing is in flight at all. No while an edit is outstanding and this is some
    /// older number — that is the one that would drag the value back.
    fn may_set(&self, value: i32) -> bool {
        let pending = self.0.get();
        if pending == value {
            self.0.set(-1);
            return true;
        }
        pending < 0
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let launched = Instant::now();

    let window = AppWindow::new()?;
    // Created up front and shown on demand: a Slint window is cheap when it is not visible,
    // and creating one from inside an event handler is a good way to find edge cases.
    let about = AboutWindow::new()?;

    // The Wayland app id, and the X11 `WM_CLASS`: what a panel, a task switcher and a
    // `.desktop` file match a window against. Slint hands it to winit only when it is set, so
    // without this line the window is anonymous — no icon next to it, and no way to pin it.
    // It has to come after the components exist (Slint has no context to set it on until then)
    // and before the window is shown, which is when the platform window is actually created.
    if let Err(problem) = slint::set_xdg_app_id("embr") {
        eprintln!("[app] could not set the XDG app id: {problem}");
    }

    let (commands, command_queue) = mpsc::channel::<Command>();
    let (states, state_queue) = mpsc::channel::<UiState>();
    let (notices, notice_queue) = mpsc::channel::<Notice>();
    let (requests, request_queue) = mpsc::channel::<UiRequest>();

    // --- the tray: registered normally, drawn only if the bar has the id pinned (see README) ---
    //
    // One place owns the set of icons, and both threads hold a clone of it: the device thread
    // decides *which* icons exist and what they say, the window flips their style the moment the
    // switch it owns moves. That split is why the switch is instant — a style change must not queue
    // behind a sweep, which is seconds of HID++ traffic.
    let tray = tray::Icons::new(commands.clone(), requests.clone());

    // --- the threads that are allowed to be slow ---
    match notify::spawn(notice_queue, requests.clone()) {
        Ok(_) => {}
        Err(problem) => eprintln!("[app] no notification thread: {problem}"),
    }
    // The window keeps its own handle so hiding it can say so out loud; the device thread gets
    // a clone of the same sender.
    match worker::spawn(command_queue, states, notices.clone(), tray.clone()) {
        Ok(_) => {}
        Err(problem) => eprintln!("[app] no device thread: {problem} — the window will stay empty"),
    }

    let models = Models::new();
    window.set_devices(ModelRc::from(models.devices.clone()));
    window.set_tray_choices(ModelRc::from(models.tray_choices.clone()));
    // The style picker: five tiles whose pictures are the tray's own pixmaps. Built with the
    // default at first — the configuration only arrives with the first snapshot, and a picker
    // with no selection at all for two seconds would be worse than one that corrects itself.
    window.set_tray_styles(ModelRc::from(models.tray_styles.clone()));
    window.set_tray_colors(ModelRc::from(models.tray_colors.clone()));
    window.set_refreshing(true);

    let latest: SharedState = Rc::new(RefCell::new(None));
    let selection: Selection = Rc::new(RefCell::new(None));
    let pushed = Pushed::new();
    // …and the picker starts out agreed with it, so the first snapshot does not rebuild five
    // tiles that are already right.
    models.show_tray_look(&window, &pushed, TrayLook::default());
    // Whether the window is on screen, and when the last sweep was asked for: the two things
    // the scheduled cadence needs (`core::scheduled_interval` decides what they mean).
    let window_open = Rc::new(Cell::new(true));
    let last_refresh = Rc::new(Cell::new(Instant::now()));
    // The window's own size, in physical pixels, as the backend reports it. Logged when it
    // changes: whether the app *sees* a resize is the difference between a layout that adapts and
    // one that draws its opening size inside a bigger frame, and that is not visible in a
    // screenshot of the frame alone.
    let last_size = Rc::new(Cell::new((0u32, 0u32)));
    // When the devices were last asked whether they are still there. Its own clock, not the
    // sweep's: a watch must not reset the sweep interval, or a five-second watch would mean the
    // configured sweep never happened.
    let last_watch = Rc::new(Cell::new(Instant::now()));
    // How many sweeps in a row could not read every device. `core::next_sweep` turns that into a
    // shorter interval, so a device that has just been picked up appears in the window within
    // seconds instead of at the next scheduled sweep.
    let attempts = Rc::new(Cell::new(0u32));
    // Device presence, straight from the kernel: a HID device appearing or disappearing is
    // something the kernel has already seen, and listening costs nothing until it happens — no
    // polling, and no waking a sleeping device to find out whether it is still asleep. Failing to
    // open this is not a failure: the app is then exactly what it was before, sweeps on a cadence.
    let uevents = match embr_core::Uevents::open() {
        Ok(uevents) => Some(uevents),
        Err(problem) => {
            eprintln!("[app] not watching for devices arriving and leaving: {problem}");
            None
        }
    };
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

    window.on_tray_style_chosen({
        let window = window.as_weak();
        let commands = commands.clone();
        let pushed = pushed.clone();
        let tray = tray.clone();
        let models = models.clone();
        move |id| {
            let Some(style) = TrayStyle::from_name(&id) else {
                // A tile whose id this build does not know: nothing to do but say so. It cannot
                // happen from the picker, which is built from `TrayStyle::ALL`.
                eprintln!("[app] unknown tray icon style \"{id}\"; the icon is unchanged");
                return;
            };
            let look = TrayLook {
                style,
                ..pushed.tray_look.get()
            };
            if look == pushed.tray_look.get() {
                // The tile that is already chosen, clicked again.
                return;
            }
            eprintln!(
                "[app] the tray icon is drawn as {} ({})",
                style.label(),
                style.as_str()
            );
            apply_look(
                &window,
                &commands,
                &pushed,
                &tray,
                &models,
                look,
                Command::SetTrayStyle(style),
            );
        }
    });

    window.on_tray_text_color_chosen({
        let window = window.as_weak();
        let commands = commands.clone();
        let pushed = pushed.clone();
        let tray = tray.clone();
        let models = models.clone();
        move |id| {
            let Some(color) = TrayTextColor::from_name(&id) else {
                eprintln!("[app] unknown tray text colour \"{id}\"; the icon is unchanged");
                return;
            };
            let look = TrayLook {
                text_color: color,
                ..pushed.tray_look.get()
            };
            if look == pushed.tray_look.get() {
                return;
            }
            eprintln!(
                "[app] the tray icon's text is drawn {} ({})",
                color.label(),
                color.as_str()
            );
            apply_look(
                &window,
                &commands,
                &pushed,
                &tray,
                &models,
                look,
                Command::SetTrayTextColor(color),
            );
        }
    });

    window.on_tray_device_toggled({
        let commands = commands.clone();
        move |key, on| {
            eprintln!(
                "[app] {} {} an icon of its own",
                key,
                if on { "gets" } else { "gives up" }
            );
            let command = Command::SetTrayDevice {
                key: key.to_string(),
                on,
            };
            if commands.send(command).is_err() {
                eprintln!("[app] the device thread has stopped; the choice was not saved");
            }
        }
    });

    window.on_close_to_tray_toggled({
        let commands = commands.clone();
        let pushed = pushed.clone();
        move |enabled| {
            if enabled == pushed.close_to_tray.get() {
                return;
            }
            pushed.close_to_tray.set(enabled);
            eprintln!(
                "[app] closing the window will {}",
                if enabled {
                    "leave Embr running"
                } else {
                    "quit"
                }
            );
            if commands.send(Command::SetCloseToTray(enabled)).is_err() {
                eprintln!("[app] the device thread has stopped; the choice was not saved");
            }
        }
    });

    window.on_watch_devices_toggled({
        let commands = commands.clone();
        let pushed = pushed.clone();
        move |enabled| {
            if enabled == pushed.watch_devices.get() {
                return;
            }
            pushed.watch_devices.set(enabled);
            eprintln!(
                "[app] devices will {} be checked between sweeps",
                if enabled { "" } else { "not" }
            );
            if commands.send(Command::SetWatchDevices(enabled)).is_err() {
                eprintln!("[app] the device thread has stopped; the choice was not saved");
            }
        }
    });

    window.on_rescan({
        let window = window.as_weak();
        let commands = commands.clone();
        let attempts = attempts.clone();
        move || {
            eprintln!("[app] rescan asked for");
            // The user asking restarts the retry sequence: a device they are looking at is worth
            // asking about in ten seconds, not in the eighty the backoff had reached.
            attempts.set(0);
            if let Some(window) = window.upgrade() {
                window.set_refreshing(true);
            }
            // The user is watching, and a device that has just been picked up needs a moment:
            // this sweep waits longer than a scheduled one before writing anything off.
            if commands.send(Command::Rescan).is_err() {
                eprintln!("[app] the device thread has stopped; nothing was rescanned");
            }
        }
    });

    window.on_refresh_seconds_changed({
        let commands = commands.clone();
        let pushed = pushed.clone();
        move |seconds| {
            let seconds = seconds.clamp(5, 600) as u64;
            pushed.refresh_pending.set(seconds as i32);
            eprintln!("[app] a fresh reading every {seconds}s while the window is open");
            if commands.send(Command::SetRefreshSeconds(seconds)).is_err() {
                eprintln!("[app] the device thread has stopped; the interval was not saved");
            }
        }
    });

    window.on_quit_requested(|| {
        eprintln!("[app] quit requested from the window");
        if let Err(problem) = slint::quit_event_loop() {
            eprintln!("[app] could not stop the event loop: {problem}");
        }
    });

    // The close button. With "close to tray" on — the default — the window goes away and the
    // app does not: the devices keep being read, the tray keeps its menu and a low battery can
    // still be announced. With it off, the close button means what it says.
    window.window().on_close_requested({
        let latest = latest.clone();
        let notices = notices.clone();
        let window_open = window_open.clone();
        let last_refresh = last_refresh.clone();
        let said_so = Rc::new(Cell::new(false));
        move || {
            let close_to_tray = latest
                .borrow()
                .as_ref()
                .is_some_and(|state| state.close_to_tray);
            match window_close_action(close_to_tray) {
                WindowClose::Hide => {
                    eprintln!("[app] window hidden; Embr keeps running in the tray");
                    // A closed window is not being watched, so the scans slow right down.
                    window_open.set(false);
                    last_refresh.set(Instant::now());
                    if !said_so.replace(true) {
                        // Nothing else on screen says the app is still there, and the tray
                        // icon is the only way back on a bar that has not pinned it.
                        dispatch_note(
                            &notices,
                            "Embr is still running",
                            "Closing the window leaves it watching in the background. Reopen it \
                             from the tray icon, or quit from the tray's menu.",
                        );
                    }
                    CloseRequestResponse::HideWindow
                }
                WindowClose::Quit => {
                    eprintln!("[app] closing Embr (close to tray is off)");
                    if let Err(problem) = slint::quit_event_loop() {
                        eprintln!("[app] could not stop the event loop: {problem}");
                    }
                    CloseRequestResponse::HideWindow
                }
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

    // The Settings page is otherwise reachable only by clicking the header button, which makes a
    // change to it awkward to check. Same shape as EMBR_DEBUG_ABOUT below, and likewise raised from
    // inside the event loop, because the view is reset whenever the window is brought up.
    if std::env::var_os("EMBR_DEBUG_SETTINGS").is_some() {
        window.set_view(1);
        let window = window.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
            if let Some(window) = window.upgrade() {
                window.set_view(1);
            }
        });
    }

    // The other half of that: the part of the settings page below the fold, which is reachable only
    // with a wheel. Held in a local — the underscore is deliberate — because a `slint::Timer`
    // stops when it is dropped.
    let _debug_scroll = match std::env::var("EMBR_DEBUG_SCROLL") {
        Ok(amount) => match amount.parse::<f32>() {
            Ok(amount) => debug_scroll(&window, amount),
            Err(problem) => {
                eprintln!("[app] EMBR_DEBUG_SCROLL is not a number ({problem}); not scrolling");
                slint::Timer::default()
            }
        },
        Err(_) => slint::Timer::default(),
    };

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
        let models = models.clone();
        let pushed = pushed.clone();
        let commands = commands.clone();
        let window_open = window_open.clone();
        let last_refresh = last_refresh.clone();
        let last_size = last_size.clone();
        let last_watch = last_watch.clone();
        let attempts = attempts.clone();
        move || {
            // 1. What the tray or a notification asked the window to do.
            while let Ok(request) = request_queue.try_recv() {
                // Taken out before the match borrows `request`: an icon that is about one device
                // opens the window on that device, because the click was about the hardware it
                // was reporting.
                let wanted = match &request {
                    UiRequest::ShowDevice(key) => Some(key.clone()),
                    _ => None,
                };
                match request {
                    UiRequest::ShowWindow | UiRequest::ShowDevice(_) => {
                        if let Some(key) = wanted {
                            *selection.borrow_mut() = Some(key);
                        }
                        if let Some(window) = window.upgrade() {
                            // The tray's item is called "Show devices", and the notification's
                            // action says the same: whoever asked for the window wants the
                            // devices, not whichever page they left it on.
                            window.set_view(0);
                            if let Err(problem) = window.show() {
                                eprintln!("[app] could not show the window: {problem}");
                            }
                            window.window().set_minimized(false);
                        }
                        // And they want today's numbers, not the ones from whenever the window
                        // was last open: a read the moment it appears, with the longer waits a
                        // device that has been asleep needs.
                        window_open.set(true);
                        last_refresh.set(Instant::now());
                        if commands.send(Command::Rescan).is_err() {
                            eprintln!("[app] the device thread has stopped; nothing was read");
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
                let incomplete = state.snapshot.incomplete();
                if let Some(window) = window.upgrade() {
                    apply(&window, &latest, &selection, &models, &pushed, state);
                }
                // A pass has just finished. The next watch is counted from here rather than from
                // the moment one was asked for: a pass that took longer than the watch interval
                // would otherwise leave a queue of watches stacked behind it.
                last_watch.set(Instant::now());
                // A sweep that could not read something is worth repeating soon; one that read
                // everything starts the count again. `core::next_sweep` decides what the count is
                // worth — this only counts.
                attempts.set(if incomplete {
                    attempts.get().saturating_add(1)
                } else {
                    0
                });
            }
            // 3. Keep the relative ages honest.
            if let Some(window) = window.upgrade() {
                tick_ages(&window, &latest, &selection, &models.devices);
            }

            // 3b. Say so when the window's size changes, so that "the layout adapted" and "the
            // frame grew around a layout stuck at its opening size" can be told apart from the
            // log instead of from a screenshot of the frame.
            if let Some(window) = window.upgrade() {
                let size = window.window().size();
                let size = (size.width, size.height);
                if size != last_size.get() {
                    eprintln!("[app] window {}x{} (physical)", size.0, size.1);
                    last_size.set(size);
                }
            }

            // 3c. Something HID-shaped arrived or left. The kernel said so; the sweep below is
            // what turns that into a device list.
            let quiet_enough = last_refresh.get().elapsed() >= SETTLE;
            let device_changed = quiet_enough
                && uevents
                    .as_ref()
                    .is_some_and(|uevents| uevents.hid_changed());
            if device_changed {
                attempts.set(0);
            }

            // 4. What to ask the devices next: a sweep when the interval is up (or when a device
            // arrived or left), and a watch pass in between — while the window is open — so that a
            // device switching off is noticed in seconds rather than at the next sweep.
            // `core::next_sweep` and `core::watch_interval` decide the numbers; this only counts.
            let seconds = latest
                .borrow()
                .as_ref()
                .map(|state| state.refresh_seconds)
                .unwrap_or(60);
            let watching = latest
                .borrow()
                .as_ref()
                .map(|state| state.watch_devices)
                .unwrap_or(true);
            let watching_now = embr_core::watch_interval(watching, window_open.get());
            let interval = embr_core::next_sweep(
                seconds,
                window_open.get(),
                attempts.get(),
                watching_now.is_some(),
            );
            let sweep_due = device_changed || last_refresh.get().elapsed() >= interval;
            let watch_due =
                watching_now.is_some_and(|interval| last_watch.get().elapsed() >= interval);

            if sweep_due {
                last_refresh.set(Instant::now());
                // A sweep asks everything a watch would, and then some.
                last_watch.set(Instant::now());
                let reason = if device_changed {
                    "[app] sweeping now: a device arrived or left"
                } else if attempts.get() > 0 {
                    "[app] sweeping now: something was not answering"
                } else {
                    "[app] scheduled sweep"
                };
                eprintln!(
                    "{reason} ({}s, window {})",
                    interval.as_secs(),
                    if window_open.get() { "open" } else { "closed" }
                );
                if let Some(window) = window.upgrade() {
                    window.set_refreshing(true);
                }
                if commands.send(Command::Refresh).is_err() {
                    eprintln!("[app] the device thread has stopped; nothing was refreshed");
                }
            } else if watch_due {
                last_watch.set(Instant::now());
                eprintln!("[app] watch pass: asking each device whether it is still there");
                // Deliberately no `set_refreshing`: this happens every few seconds and the
                // window should not announce a scan that the user did not ask for.
                if commands.send(Command::Watch).is_err() {
                    eprintln!("[app] the device thread has stopped; the devices were not watched");
                }
            }
        }
    });

    window.show()?;
    eprintln!(
        "[app] window shown {} ms after launch",
        launched.elapsed().as_millis()
    );
    // `run_event_loop_until_quit` rather than `run_event_loop`: with close-to-tray on, hiding
    // the window is not quitting, and `run_event_loop` would end the program the moment the
    // last window is hidden (it only knows about windows and Slint's own tray icon, and the
    // tray here belongs to another thread). Every way out calls `quit_event_loop`.
    if let Err(problem) = slint::run_event_loop_until_quit() {
        eprintln!("[app] the event loop ended with {problem}");
    }
    eprintln!("[app] the event loop has returned; exiting");
    Ok(())
}

/// Put one look into effect: the icons themselves first (the tray is one D-Bus round trip away,
/// and a style change must not queue behind a sweep — measured: 827 ms with a sweep in flight, 29 ms
/// without), then the window, then the choice on its way to the configuration file.
fn apply_look(
    window: &slint::Weak<AppWindow>,
    commands: &std::sync::mpsc::Sender<Command>,
    pushed: &Pushed,
    tray: &tray::Icons,
    models: &Models,
    look: TrayLook,
    command: Command,
) {
    tray.restyle(look);
    if let Some(window) = window.upgrade() {
        models.show_tray_look(&window, pushed, look);
    } else {
        pushed.tray_look.set(look);
    }
    if commands.send(command).is_err() {
        eprintln!("[app] the device thread has stopped; the choice was not saved");
    }
}

/// `EMBR_DEBUG_SCROLL`: scroll whatever is showing, so a page that continues below the fold can be
/// photographed on a desktop where nothing can inject a wheel event. The event goes through
/// `dispatch_event` — the entry point the backend itself uses — so only the transport leg, the
/// compositor handing over a wheel event, is skipped. The returned timer has to be kept alive.
fn debug_scroll(window: &AppWindow, amount: f32) -> slint::Timer {
    let window = window.as_weak();
    let ticks = Rc::new(Cell::new(0u32));
    let counter = ticks.clone();
    let timer = slint::Timer::default();
    // Started late: events delivered while the window is still being mapped are dropped.
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(250),
        move || {
            let round = counter.get();
            counter.set(round + 1);
            // The first seconds belong to the window being mapped: events sent before it is up
            // are dropped by the platform, so the scrolling starts once it is.
            if round < 16 {
                return;
            }
            if round >= 36 {
                if round == 36 {
                    eprintln!("[app] EMBR_DEBUG_SCROLL: 20 scroll events of {amount} delivered");
                }
                return;
            }
            let Some(window) = window.upgrade() else {
                return;
            };
            window
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(400.0, 500.0),
                    delta_x: 0.0,
                    delta_y: amount,
                });
        },
    );
    timer
}

/// Take one snapshot into the window: the list, the detail card and the settings.
fn apply(
    window: &AppWindow,
    latest: &SharedState,
    selection: &Selection,
    models: &Models,
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

    models.devices.set_vec(
        state
            .snapshot
            .devices
            .iter()
            .map(view::device)
            .collect::<Vec<_>>(),
    );

    // One row per device on the settings page: the devices the user can give an icon of their own.
    models.tray_choices.set_vec(
        state
            .snapshot
            .devices
            .iter()
            .map(|device| view::tray_choice(device, &state.tray_devices))
            .collect::<Vec<_>>(),
    );

    window.set_thresholds_text(state.thresholds_text.clone().into());
    window.set_config_path(state.config_path.clone().into());
    window.set_notice(state.notice.clone().unwrap_or_default().into());
    window.set_snooze_caption(messages::snooze_caption(state.snooze_minutes).into());
    pushed.notifications.set(state.notifications_enabled);
    window.set_notifications_enabled(state.notifications_enabled);
    // The picker is only rebuilt when the style actually changed — otherwise every snapshot would
    // make five new preview images to draw the same five pictures.
    if pushed.tray_look.get() != state.tray_look {
        models.show_tray_look(window, pushed, state.tray_look);
    }
    pushed.close_to_tray.set(state.close_to_tray);
    window.set_close_to_tray(state.close_to_tray);
    pushed.watch_devices.set(state.watch_devices);
    window.set_watch_devices(state.watch_devices);
    apply_alert_level(window, &state, selection, pushed);
    apply_refresh_interval(window, &state, pushed);
    window.set_refreshing(false);

    *latest.borrow_mut() = Some(state);
    tick_ages(window, latest, selection, &models.devices);
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

    let pending = &pushed.alert_pending;
    if !pending.may_set(level) {
        // A snapshot from before the user's edit: it must not drag the box back.
        return;
    }
    if window.get_alert_level() != level {
        window.set_alert_level(level);
    }
    window.set_alert_custom(custom);
}

/// The same, for the interval the window refreshes itself at while it is open.
fn apply_refresh_interval(window: &AppWindow, state: &UiState, pushed: &Pushed) {
    let seconds = i32::try_from(state.refresh_seconds).unwrap_or(60);
    if pushed.refresh_pending.may_set(seconds) && window.get_refresh_seconds() != seconds {
        window.set_refresh_seconds(seconds);
    }
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
