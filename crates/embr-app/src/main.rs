//! Embr's window, in GTK 4 + libadwaita.
//!
//! A battery monitor for Logitech devices, in GTK 4 + libadwaita. Everything below the window is
//! shared and toolkit-free: `embr-core` decides what anything means, `embr-hidpp` talks to the
//! hardware, and `embr-desktop` owns the tray icons and the notifications (D-Bus), so the window
//! layer holds no D-Bus code and one function decides what a tray icon looks like.
//!
//! Threads: the UI thread owns every widget and never opens a device; the
//! device thread ([`worker`]) owns `core::Monitor` and never touches a widget; the notification
//! thread owns `notify-rust`; the tray belongs to `ksni`. What crosses between them is
//! [`Command`], [`State`], [`UiEvent`] and `embr_desktop::TrayAction` — never a widget and never a
//! device handle.

mod messages;
mod page;
mod rail;
mod settings;
mod widgets;
mod worker;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use adw::prelude::*;
use gtk4::glib;

use embr_core::{FeaturePage, TrayStyle, WindowClose, summary_text, window_close_action};
use embr_desktop::tray::Icons;
use embr_desktop::{Notice, TrayAction, dispatch_note, notify};

use messages::{Command, State, UiEvent, dispatch};
use page::Content;
use rail::Rail;
use settings::Settings;

/// The shortest gap between a sweep and another one forced by a device arriving or leaving. A
/// dongle announces itself as several devices within a few milliseconds and the socket is drained
/// in one call, so that is a single sweep; this is for the device stuck re-enumerating.
const SETTLE: Duration = Duration::from_secs(2);

/// How often the UI thread looks at what the device thread and the user have produced. Four times
/// a second is under the flicker threshold and costs nothing: the tick drains two channels and
/// writes a handful of labels.
const TICK: Duration = Duration::from_millis(250);

/// A page to open by itself, for looking at one that would otherwise need a click:
/// `EMBR_DEBUG_PAGE=lighting embr`.
const DEBUG_PAGE_VARIABLE: &str = "EMBR_DEBUG_PAGE";

/// A device to open by itself, matched on its name, for looking at a device that is not the one
/// the window would choose: `EMBR_DEBUG_DEVICE=G913 embr`.
const DEBUG_DEVICE_VARIABLE: &str = "EMBR_DEBUG_DEVICE";

/// The About dialog, which is otherwise reachable only by clicking its button:
/// `EMBR_DEBUG_ABOUT=1 embr`.
const DEBUG_ABOUT_VARIABLE: &str = "EMBR_DEBUG_ABOUT";

/// The settings page, which is otherwise reachable only by clicking the gear:
/// `EMBR_DEBUG_SETTINGS=1 embr`.
const DEBUG_SETTINGS_VARIABLE: &str = "EMBR_DEBUG_SETTINGS";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id("io.github.jensend6965.Embr")
        .build();
    app.connect_activate(build);
    app.run()
}

fn build(app: &adw::Application) {
    let launched = Instant::now();

    let (commands, command_queue) = mpsc::channel::<Command>();
    let (states, state_queue) = mpsc::channel::<State>();
    let (notices, notice_queue) = mpsc::channel::<Notice>();
    // The tray and the notification thread reach the window through these two, and never by
    // touching a widget: a click arrives as a `TrayAction` on a channel the tick drains.
    let (tray_actions, tray_action_queue) = mpsc::channel::<TrayAction>();
    // Clicks in the rail and the settings page, handled on this thread's own tick so that the
    // device the window shows changes in exactly one place.
    let (events, event_queue) = mpsc::channel::<UiEvent>();

    // --- the tray: registered normally, drawn only if the bar has the ids pinned (see README) ---
    //
    // One place owns the set of icons, and both threads hold a clone of it: the device thread
    // decides *which* icons exist and what they say, the window flips their style the moment the
    // picker moves. That split is why a style change is instant — it must not queue behind a
    // sweep, which is seconds of HID++ traffic.
    let tray = Icons::new(tray_actions.clone());

    match notify::spawn(notice_queue, tray_actions.clone()) {
        Ok(_) => {}
        Err(problem) => {
            eprintln!("[app] no notification thread: {problem} — nothing will be announced")
        }
    }
    match worker::spawn(command_queue, states, notices.clone(), tray.clone()) {
        Ok(_) => {}
        Err(problem) => eprintln!("[app] no device thread: {problem} — the window will stay empty"),
    }

    // The reading colours, before anything is drawn with them.
    crate::widgets::install_level_css();

    // Open at a size that fits the screen it opens on. Asking for a fixed size is how a window ends
    // up taller than the desktop: the header stays reachable and the bottom of the pane is simply
    // below the screen edge, with no way to scroll it into view.
    let (width, height) = default_window_size();
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Embr")
        .default_width(width)
        .default_height(height)
        .build();

    // ---- the header: what the devices add up to, Rescan, the gear, and About ----
    let header = adw::HeaderBar::new();
    let rescan = gtk4::Button::builder().tooltip_text("Rescan").build();
    rescan.add_css_class("suggested-action");
    let rescan_icon = gtk4::Image::from_icon_name("view-refresh-symbolic");
    let spinner = gtk4::Spinner::new();
    rescan.set_child(Some(&rescan_icon));
    let gear = gtk4::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Settings")
        .build();
    let about_button = gtk4::Button::builder()
        .icon_name("help-about-symbolic")
        .tooltip_text("About Embr")
        .build();
    let summary = gtk4::Label::builder().label("Embr").build();
    summary.add_css_class("heading");
    let subsummary = gtk4::Label::builder().label("").build();
    subsummary.add_css_class("caption");
    subsummary.add_css_class("dim-label");
    let titles = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    titles.append(&summary);
    titles.append(&subsummary);
    header.pack_start(&rescan);
    header.pack_end(&about_button);
    header.pack_end(&gear);
    header.set_title_widget(Some(&titles));

    // ---- the body: the rail, and the pages of the device it picked ----
    let rail = Rc::new(Rail::new(events.clone()));
    let content = Rc::new(Content::new(commands.clone()));
    // A paned rather than a box with a separator between two children: the divider's position *is*
    // the rail's width, so a long device name cannot widen the column and choosing a device cannot
    // move it. `resize_start_child(false)` holds that width while the window grows — the pages take
    // the extra room — and the divider is still there to be dragged when the user wants it wider.
    let body = gtk4::Paned::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .start_child(&rail.container)
        .end_child(&content.container)
        .position(rail::WIDTH)
        .resize_start_child(false)
        .shrink_start_child(false)
        .wide_handle(true)
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&body));
    window.set_content(Some(&toolbar));

    let settings = Rc::new(Settings::new(commands.clone(), events.clone()));

    let about = adw::AboutDialog::builder()
        .application_name("Embr")
        .application_icon("embr")
        .version(env!("CARGO_PKG_VERSION"))
        .comments(
            "A battery monitor for Logitech devices: what is attached to this machine, how full \
             it is, and a tray icon that says so at a glance.",
        )
        .website("https://github.com/jensend6965/embr")
        .build();

    gear.connect_clicked({
        let settings = settings.clone();
        let window = window.clone();
        move |_| settings.present(&window)
    });
    about_button.connect_clicked({
        let about = about.clone();
        let window = window.clone();
        move |_| about.present(Some(&window))
    });

    // ---- what the window is showing, and what it has asked about ----
    let latest: Rc<RefCell<Option<State>>> = Rc::new(RefCell::new(None));
    let selected: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    // The last tab the pane was *seen* showing, for the one log line a switch deserves. Not an
    // instruction: which page is open belongs to the pane and, through it, to the user — the tick
    // reads it back and never sets it, which is what keeps a reading arriving mid-click from
    // moving the user off the page they picked.
    let shown_page: Rc<RefCell<FeaturePage>> = Rc::new(RefCell::new(FeaturePage::Battery));
    // The devices whose feature set has been asked for. A feature set is an inventory rather than
    // a reading, so it is asked for once per device and again only when the user rescans.
    let asked: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));
    let refreshing = Rc::new(Cell::new(false));
    let spin_shown = Rc::new(Cell::new(false));
    let last_refresh = Rc::new(Cell::new(Instant::now()));
    let last_watch = Rc::new(Cell::new(Instant::now()));
    let attempts = Rc::new(Cell::new(0u32));
    let last_size = Rc::new(Cell::new((0u32, 0u32)));
    // Whether the window is on screen. With close-to-tray on it can be hidden while the app keeps
    // running, and `core::scheduled_interval`/`core::watch_interval` decide what that means: no
    // watch passes, and a two-minute floor on the sweeps.
    let window_open = Rc::new(Cell::new(true));

    // DEVICE presence, straight from the kernel: a HID device appearing or leaving is something
    // the kernel has already seen, and listening costs nothing until it happens. Failing to open
    // this is not a failure — the window is then what it was, sweeps on a cadence.
    let uevents = match embr_core::Uevents::open() {
        Ok(uevents) => Some(uevents),
        Err(problem) => {
            eprintln!("[app] not watching for devices arriving and leaving: {problem}");
            None
        }
    };

    rescan.connect_clicked({
        let commands = commands.clone();
        let refreshing = refreshing.clone();
        let asked = asked.clone();
        let attempts = attempts.clone();
        move |_| {
            eprintln!("[app] rescan asked for");
            // The shelf of feature sets is read again too: a device that was asleep when it was
            // first asked is given a second chance by the sweep the user just demanded.
            asked.borrow_mut().clear();
            attempts.set(0);
            refreshing.set(true);
            dispatch(&commands, Command::Rescan);
        }
    });

    // ---- the close button ----
    //
    // With close-to-tray on (the default) the close button hides the window and the app keeps
    // running: the device thread keeps sweeping, the tray keeps its menu and a low battery is
    // still announced. With it off, the close button means what it says.
    window.connect_close_request({
        let app = app.clone();
        let window = window.clone();
        let latest = latest.clone();
        let notices = notices.clone();
        let window_open = window_open.clone();
        let last_refresh = last_refresh.clone();
        let said_so = Rc::new(Cell::new(false));
        move |_| {
            let close_to_tray = latest
                .borrow()
                .as_ref()
                .is_some_and(|state| state.close_to_tray);
            match window_close_action(close_to_tray) {
                WindowClose::Hide => {
                    eprintln!("[app] window hidden; Embr keeps running in the tray");
                    // A hidden window is not being watched, so the scans slow right down.
                    window_open.set(false);
                    last_refresh.set(Instant::now());
                    if !said_so.replace(true) {
                        // Nothing else on screen says the app is still there, and the tray icon is
                        // the only way back on a bar that has not pinned it.
                        dispatch_note(
                            &notices,
                            "Embr is still running",
                            "Closing the window leaves it watching in the background. Reopen it \
                             from the tray icon, or quit from the tray's menu.",
                        );
                    }
                    window.set_visible(false);
                    glib::Propagation::Stop
                }
                WindowClose::Quit => {
                    eprintln!("[app] closing Embr (close to tray is off)");
                    app.quit();
                    glib::Propagation::Stop
                }
            }
        }
    });

    glib::timeout_add_local(TICK, {
        let rail = rail.clone();
        let content = content.clone();
        let settings = settings.clone();
        let commands = commands.clone();
        let tray = tray.clone();
        let app = app.clone();
        let window = window.clone();
        let latest = latest.clone();
        let selected = selected.clone();
        let shown_page = shown_page.clone();
        let asked = asked.clone();
        let refreshing = refreshing.clone();
        let spin_shown = spin_shown.clone();
        let attempts = attempts.clone();
        let last_refresh = last_refresh.clone();
        let last_watch = last_watch.clone();
        let last_size = last_size.clone();
        let window_open = window_open.clone();
        // The device asked for by name, if any, so the window can be opened on one device.
        let debug_device: Option<String> = std::env::var(DEBUG_DEVICE_VARIABLE).ok();
        move || {
            // 1. What the user did — a click in the rail or on the settings page.
            while let Ok(event) = event_queue.try_recv() {
                match event {
                    UiEvent::Select(key) => {
                        eprintln!("[app] showing {key}");
                        *selected.borrow_mut() = Some(key);
                    }
                    UiEvent::RestyleTray(look) => {
                        // The icons themselves first: the tray is one D-Bus round trip away, and
                        // this must not wait for a sweep. The configuration is written by the
                        // command that comes with it.
                        tray.restyle(look);
                        settings.show_look(look);
                    }
                    UiEvent::Quit => {
                        eprintln!("[app] quit requested from the settings page");
                        app.quit();
                    }
                }
            }

            // 2. What the tray or a notification asked for. A tray click is user input like any
            // other, and it arrives on a channel rather than as a call into this thread.
            while let Ok(action) = tray_action_queue.try_recv() {
                match action {
                    TrayAction::Rescan => {
                        eprintln!("[app] rescan asked for from the tray");
                        asked.borrow_mut().clear();
                        attempts.set(0);
                        refreshing.set(true);
                        dispatch(&commands, Command::Rescan);
                    }
                    TrayAction::ShowWindow { key } => {
                        // An icon that is about one device opens the window on *that* device: the
                        // click was about the hardware it was reporting.
                        if let Some(key) = key {
                            *selected.borrow_mut() = Some(key);
                        }
                        window.present();
                        window_open.set(true);
                        last_refresh.set(Instant::now());
                        eprintln!("[app] the window was asked for; reading the devices again");
                        dispatch(&commands, Command::Rescan);
                    }
                    TrayAction::Quit => {
                        eprintln!("[app] quit requested from the tray");
                        app.quit();
                    }
                }
            }

            // 3. What the device thread has produced since the last tick.
            while let Ok(state) = state_queue.try_recv() {
                let incomplete = state.snapshot.incomplete();
                settings.apply(&state);
                *latest.borrow_mut() = Some(state);
                refreshing.set(false);
                // A pass has just finished: the next watch is counted from here rather than from
                // the moment one was asked for, so a slow pass cannot leave a queue of watches
                // stacked behind it.
                last_watch.set(Instant::now());
                attempts.set(if incomplete {
                    attempts.get().saturating_add(1)
                } else {
                    0
                });
            }

            // 4. Draw it: the rail, then the page the rail picked.
            let now = SystemTime::now();
            let borrowed = latest.borrow();
            let drawn = borrowed.as_ref().map(|state| state.snapshot.clone());
            let notice = borrowed.as_ref().and_then(|state| state.notice.clone());
            let tray_style = borrowed
                .as_ref()
                .map(|state| state.tray_look.style)
                .unwrap_or_default();
            drop(borrowed);

            let drawn_state = latest.borrow().clone();
            if let (Some(snapshot), Some(state)) = (drawn.as_ref(), drawn_state.as_ref()) {
                // The device that was open is still there, or the worst one takes its place: a
                // card that was opened must not leave the page showing a device that has left.
                let wanted = selected.borrow().clone();
                let chosen = wanted
                    .filter(|key| rail.has(key))
                    .or_else(|| snapshot.devices.first().map(|device| device.key.clone()));
                // A device asked for by name wins over all of that, so a window can be opened on
                // a device that is not the one the batteries would choose.
                let chosen = debug_device
                    .as_ref()
                    .and_then(|needle| {
                        snapshot
                            .devices
                            .iter()
                            .find(|device| device.name.contains(needle.as_str()))
                            .map(|device| device.key.clone())
                    })
                    .or(chosen);
                let device = snapshot
                    .devices
                    .iter()
                    .find(|device| Some(&device.key) == chosen.as_ref());
                *selected.borrow_mut() = chosen.clone();

                rail.sync(snapshot, chosen.as_deref(), now);
                content.sync(device, state, now);
                // The tabs belong to the user, so this reads them rather than steering them:
                // whatever the strip is showing is what the window now considers open, and a
                // click on a tab therefore survives every snapshot that follows it. Said out loud
                // because a tab switch is otherwise the one thing here with no trace.
                if let Some(visible) = chosen.as_deref().and_then(|key| content.visible_page(key))
                    && *shown_page.borrow() != visible
                {
                    eprintln!("[app] showing the {} page", visible.label());
                    *shown_page.borrow_mut() = visible;
                }

                // A device whose feature set has not been read yet is asked about once. This is
                // what fills the rail under the open device, and it is why the rail can only
                // offer pages the hardware really has.
                if let Some(device) = device
                    && device.pages.is_none()
                    && asked.borrow_mut().insert(device.key.clone())
                {
                    eprintln!("[app] asking {} which features it has", device.name);
                    dispatch(&commands, Command::Pages(device.key.clone()));
                }

                summary.set_label(&summary_text(snapshot));
                // The line under it: whatever the configuration could not do, or what the bar
                // needs to be told to draw this icon.
                subsummary.set_label(&notice.unwrap_or_else(|| tray_hint(tray_style)));
            }

            // 5. The ages: every tick, for what is already drawn.
            if let Some(state) = latest.borrow().as_ref() {
                rail.refresh_ages(&state.snapshot, now);
                let open = selected.borrow().clone();
                let device = state
                    .snapshot
                    .devices
                    .iter()
                    .find(|device| Some(&device.key) == open.as_ref());
                content.refresh_ages(device, &state.snapshot, now);
            }
            show_refreshing(
                &rescan,
                &rescan_icon,
                &spinner,
                &spin_shown,
                refreshing.get(),
            );

            // 6. The window's own size, so that "the layout adapted" and "the frame grew around a
            // layout that did not" can be told apart from the log instead of a screenshot.
            let size = (window.width().max(0) as u32, window.height().max(0) as u32);
            if size != last_size.get() {
                eprintln!("[app] window {}x{}", size.0, size.1);
                last_size.set(size);
            }

            // 7. What to ask the devices next: a sweep when the interval is up (or when something
            // arrived or left), and a watch pass in between. `core::next_sweep` and
            // `core::watch_interval` decide the numbers; this only counts.
            let quiet_enough = last_refresh.get().elapsed() >= SETTLE;
            let device_changed = quiet_enough
                && uevents
                    .as_ref()
                    .is_some_and(|uevents| uevents.hid_changed());
            if device_changed {
                attempts.set(0);
            }

            let (seconds, watching) = latest
                .borrow()
                .as_ref()
                .map(|state| (state.refresh_seconds, state.watch_devices))
                .unwrap_or((60, true));
            let open = window_open.get();
            let watching_now = embr_core::watch_interval(watching, open);
            let interval =
                embr_core::next_sweep(seconds, open, attempts.get(), watching_now.is_some());
            let sweep_due = device_changed || last_refresh.get().elapsed() >= interval;
            let watch_due =
                watching_now.is_some_and(|interval| last_watch.get().elapsed() >= interval);

            if sweep_due {
                last_refresh.set(Instant::now());
                // A sweep asks everything a watch would, and then some.
                last_watch.set(Instant::now());
                eprintln!(
                    "[app] {} ({}s, window {})",
                    if device_changed {
                        "sweeping now: a device arrived or left"
                    } else if attempts.get() > 0 {
                        "sweeping now: something was not answering"
                    } else {
                        "scheduled sweep"
                    },
                    interval.as_secs(),
                    if open { "open" } else { "closed" }
                );
                refreshing.set(true);
                dispatch(&commands, Command::Refresh);
            } else if watch_due {
                last_watch.set(Instant::now());
                eprintln!("[app] watch pass: asking each device whether it is still there");
                dispatch(&commands, Command::Watch);
            }

            glib::ControlFlow::Continue
        }
    });

    // `EMBR_DEBUG_PAGE`: open a page that would otherwise need a click. A page the device does
    // not have is not a page — the request waits for the feature set to arrive rather than
    // showing something the hardware cannot do.
    if let Ok(id) = std::env::var(DEBUG_PAGE_VARIABLE) {
        match FeaturePage::from_id(&id) {
            Some(page) => {
                let content = content.clone();
                let latest = latest.clone();
                let waiting = Cell::new(true);
                glib::timeout_add_local(Duration::from_millis(500), move || {
                    let known = latest
                        .borrow()
                        .as_ref()
                        .and_then(|state| state.snapshot.devices.first().cloned())
                        .and_then(|device| device.pages)
                        .is_some_and(|pages| pages.contains(&page));
                    if !known {
                        return glib::ControlFlow::Continue;
                    }
                    if waiting.replace(false) {
                        eprintln!(
                            "[app] {DEBUG_PAGE_VARIABLE}={id}: opening the {} page",
                            page.label()
                        );
                        // The one place outside the pane that moves the view, and it says so:
                        // nothing in the tick ever calls this.
                        content.request_page(page);
                    }
                    glib::ControlFlow::Continue
                });
            }
            None => eprintln!("[app] {DEBUG_PAGE_VARIABLE}=\"{id}\" is not a page this build has"),
        }
    }

    if std::env::var_os(DEBUG_ABOUT_VARIABLE).is_some() {
        let about = about.clone();
        let window = window.clone();
        glib::timeout_add_local_once(Duration::from_millis(500), move || {
            eprintln!("[app] {DEBUG_ABOUT_VARIABLE} is set: opening About");
            about.present(Some(&window));
        });
    }

    if std::env::var_os(DEBUG_SETTINGS_VARIABLE).is_some() {
        let settings = settings.clone();
        let window = window.clone();
        // Raised from inside the event loop: showing a window before the loop starts leaves it
        // behind the main one, where a screenshot finds the desktop instead.
        glib::timeout_add_local_once(Duration::from_millis(500), move || {
            eprintln!("[app] {DEBUG_SETTINGS_VARIABLE} is set: opening the settings page");
            settings.present(&window);
        });
    }

    window.present();
    eprintln!(
        "[app] window shown {} ms after launch",
        launched.elapsed().as_millis()
    );
}

/// The line under the header when there is nothing to warn about: what the bar needs to know to
/// draw this icon. On a bar that draws only the items it has pinned (Noctalia does), an icon that
/// is registered but not pinned is invisible — and saying so costs one line.
fn tray_hint(style: TrayStyle) -> String {
    format!(
        "tray icon: {} — registered as \"embr\"; pin that id in your bar to see it",
        style.label()
    )
}

/// Show that a sweep is in flight in the button that asked for it, and take it back when the
/// readings arrive. A click that looks dead for the seconds a sweep takes is the thing this
/// exists to avoid.
fn show_refreshing(
    button: &gtk4::Button,
    icon: &gtk4::Image,
    spinner: &gtk4::Spinner,
    shown: &Cell<bool>,
    refreshing: bool,
) {
    if refreshing == shown.get() {
        return;
    }
    shown.set(refreshing);
    if refreshing {
        spinner.start();
        button.set_child(Some(spinner));
    } else {
        spinner.stop();
        button.set_child(Some(icon));
    }
}

/// The window's opening size: what the layout is comfortable at, cut down to fit the screen.
///
/// The rail is a fixed 340 px and the pane wants room for a two-column row, so 1240×860 is the size
/// the layout is designed for. On anything smaller — a 1366×768 laptop, a tiled window — that size
/// is refused and the app opens with its bottom edge off the desktop, so the request is capped at
/// nine tenths of the monitor's geometry, in the same logical pixels GTK lays out in.
fn default_window_size() -> (i32, i32) {
    const WANTED: (i32, i32) = (1240, 860);
    /// Below this the two-pane layout stops being usable at all; better a window that is too big
    /// for a tiny screen than one with no room for its own controls.
    const FLOOR: (i32, i32) = (720, 520);

    let Some(display) = gtk4::gdk::Display::default() else {
        return WANTED;
    };
    let Some(monitor) = display
        .monitors()
        .item(0)
        .and_then(|item| item.downcast::<gtk4::gdk::Monitor>().ok())
    else {
        return WANTED;
    };
    let area = monitor.geometry();
    let size = (
        fit(WANTED.0, area.width(), FLOOR.0),
        fit(WANTED.1, area.height(), FLOOR.1),
    );
    eprintln!(
        "[app] opening at {}x{} (screen {}x{})",
        size.0,
        size.1,
        area.width(),
        area.height()
    );
    size
}

/// One axis of [`default_window_size`]: the size asked for, capped to nine tenths of what the screen
/// has, but never below the floor the layout needs.
fn fit(want: i32, have: i32, floor: i32) -> i32 {
    want.min((have * 9 / 10).max(floor))
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn a_window_fits_the_screen_it_opens_on() {
        // This desktop: the wanted size fits with room to spare, so it is left alone.
        assert_eq!(fit(1240, 2560, 720), 1240);
        assert_eq!(fit(860, 1080, 520), 860);
        // A 1366x768 laptop: nine tenths, so the bottom edge lands on the desktop.
        assert_eq!(fit(1240, 1366, 720), 1229);
        assert_eq!(fit(860, 768, 520), 691);
        // Smaller than the floor: a slightly too big window beats one with no room for its controls.
        assert_eq!(fit(1240, 640, 720), 720);
        assert_eq!(fit(860, 400, 520), 520);
    }
}
