//! The settings window: everything that belongs to the application rather than to one device.
//!
//! The device's own setting (its low-battery level) is on the device's Battery page; this page is
//! the app's — the tray icon, notifications, the cadence, the close button and where the config
//! lives. Every control here writes through `core` and nothing else: the page sends a [`Command`]
//! and the device thread is the only thing that touches the configuration file.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::Sender;

use adw::prelude::*;
use gtk4::{Align, Orientation};

use embr_core::{Theme, TrayLook, TrayStyle, TrayTextColor, snooze_caption, tray_style_caption};

use crate::messages::{Command, State, UiEvent, dispatch, dispatch_event};
use crate::widgets;

/// The values this page has pushed into controls the user can also move.
///
/// A `Switch` fires when its value changes, including when *we* change it, so without this a
/// snapshot arriving with the old value would read as a user action and write it back. "What is
/// still in flight?" is what tells the user's edit from our own echo; `-1` means nothing is.
#[derive(Clone)]
struct Pushed {
    tray_look: Rc<Cell<TrayLook>>,
    theme: Rc<Cell<Theme>>,
    /// The three readout switches. They are pushed *and* user-written, so each needs to know what
    /// the window last put into it — otherwise setting the switch here reads as a user action and
    /// writes the same value back on every snapshot.
    show_voltage: Rc<Cell<bool>>,
    level_colors: Rc<Cell<bool>>,
    mark_estimates: Rc<Cell<bool>>,
    notifications: Rc<Cell<bool>>,
    watch_devices: Rc<Cell<bool>>,
    close_to_tray: Rc<Cell<bool>>,
    refresh: Rc<Cell<i32>>,
}

impl Pushed {
    fn new() -> Self {
        Self {
            tray_look: Rc::new(Cell::new(TrayLook::default())),
            theme: Rc::new(Cell::new(Theme::default())),
            show_voltage: Rc::new(Cell::new(true)),
            level_colors: Rc::new(Cell::new(true)),
            mark_estimates: Rc::new(Cell::new(true)),
            notifications: Rc::new(Cell::new(false)),
            watch_devices: Rc::new(Cell::new(true)),
            close_to_tray: Rc::new(Cell::new(true)),
            refresh: Rc::new(Cell::new(-1)),
        }
    }
}

/// The settings window, and the controls a snapshot has to be written into.
pub struct Settings {
    /// The dialog the window presents. `AdwPreferencesDialog` rather than the deprecated
    /// `AdwPreferencesWindow`: the dialog is what libadwaita has asked for since 1.6, and it is
    /// presented *on* the window it belongs to.
    pub dialog: adw::PreferencesDialog,
    notice: adw::ActionRow,
    notice_group: adw::PreferencesGroup,
    styles: gtk4::Box,
    style_caption: gtk4::Label,
    colours: gtk4::Box,
    colours_group: adw::PreferencesGroup,
    devices_group: adw::PreferencesGroup,
    /// The three colour-scheme tiles, and the sentence under them.
    themes: gtk4::Box,
    theme_caption: gtk4::Label,
    show_voltage: adw::SwitchRow,
    level_colors: adw::SwitchRow,
    mark_estimates: adw::SwitchRow,
    /// The device keys the per-device switches were built for, so they are only rebuilt when the
    /// set of devices changes rather than on every snapshot.
    devices: RefCell<Vec<String>>,
    /// The per-device switches that are in the group right now, so they can be taken away again.
    device_rows: RefCell<Vec<adw::SwitchRow>>,
    notifications: adw::SwitchRow,
    thresholds: adw::ActionRow,
    snooze: adw::ActionRow,
    refresh: adw::SpinRow,
    watch: adw::SwitchRow,
    close_to_tray: adw::SwitchRow,
    config: adw::ActionRow,
    pushed: Pushed,
    commands: Sender<Command>,
}

impl Settings {
    pub fn new(commands: Sender<Command>, events: Sender<UiEvent>) -> Self {
        let pushed = Pushed::new();
        let page = adw::PreferencesPage::new();

        // The notice is one row rather than a banner: an `AdwPreferencesPage` takes preference
        // groups and nothing else, and a banner in one is a type error, which is how this was
        // found. It is hidden until there is something to say.
        let notice = widgets::row("Notice");
        notice.set_visible(false);
        let notice_group = adw::PreferencesGroup::builder().build();
        notice_group.add(&notice);
        page.add(&notice_group);

        // ---- appearance: what the window follows, now that libadwaita can follow it ----
        let appearance = adw::PreferencesGroup::builder()
            .title("Appearance")
            .description(
                "A GTK application gets the desktop's colour scheme and accent for free; this is \
                 the escape hatch for one that publishes something you do not want.",
            )
            .build();
        let theme_caption = gtk4::Label::builder()
            .label("")
            .xalign(0.0)
            .wrap(true)
            .build();
        theme_caption.add_css_class("caption");
        theme_caption.add_css_class("dim-label");
        let themes = gtk4::Box::new(Orientation::Horizontal, 0);
        themes.set_valign(Align::Center);
        themes.add_css_class("linked");
        for theme in Theme::ALL {
            let button = gtk4::ToggleButton::with_label(theme.label());
            button.set_active(theme == Theme::default());
            button.connect_clicked({
                let commands = commands.clone();
                let pushed = pushed.clone();
                move |button| {
                    // GTK's linked toggles are not exclusive by themselves: the chosen one cannot
                    // be switched off again, so every other tile is told.
                    unpress_siblings(&button.parent(), button);
                    theme_chosen(theme, &commands, &pushed);
                }
            });
            themes.append(&button);
        }
        let theme_row = widgets::row("Colour scheme");
        theme_row.add_suffix(&themes);
        appearance.add(&theme_row);
        let theme_note = widgets::row("What that means");
        theme_note.add_suffix(&theme_caption);
        appearance.add(&theme_note);

        page.add(&appearance);

        // ---- what a reading says and how it is drawn: three switches, each a real choice ----
        let readouts = adw::PreferencesGroup::builder()
            .title("Readouts")
            .description("How a battery level is written and drawn, for every device at once.")
            .build();

        let level_colors = adw::SwitchRow::builder()
            .title("Colour the level bars")
            .subtitle("green when full, amber in the middle, red when low — the same bands the tray icon's level colour uses")
            .build();
        level_colors.connect_active_notify({
            let commands = commands.clone();
            let pushed = pushed.clone();
            move |row| {
                let colors = row.is_active();
                if colors == pushed.level_colors.get() {
                    return;
                }
                pushed.level_colors.set(colors);
                eprintln!(
                    "[app] the level bars are {} coloured by their level",
                    if colors { "" } else { "no longer" }
                );
                dispatch(&commands, Command::SetLevelColors(colors));
            }
        });
        readouts.add(&level_colors);

        let show_voltage = adw::SwitchRow::builder()
            .title("Show the voltage")
            .subtitle("beside the percentage, and on a device's page")
            .build();
        show_voltage.connect_active_notify({
            let commands = commands.clone();
            let pushed = pushed.clone();
            move |row| {
                let show = row.is_active();
                if show == pushed.show_voltage.get() {
                    return;
                }
                pushed.show_voltage.set(show);
                eprintln!(
                    "[app] the voltage is {} beside the percentage",
                    if show { "shown" } else { "no longer shown" }
                );
                dispatch(&commands, Command::SetShowVoltage(show));
            }
        });
        readouts.add(&show_voltage);

        let mark_estimates = adw::SwitchRow::builder()
            .title("Mark an inferred percentage")
            .subtitle("a reading worked out from voltage is written ~55% rather than 55%")
            .build();
        mark_estimates.connect_active_notify({
            let commands = commands.clone();
            let pushed = pushed.clone();
            move |row| {
                let mark = row.is_active();
                if mark == pushed.mark_estimates.get() {
                    return;
                }
                pushed.mark_estimates.set(mark);
                eprintln!(
                    "[app] a percentage read from voltage is {} marked with a ~",
                    if mark { "" } else { "no longer" }
                );
                dispatch(&commands, Command::SetMarkEstimates(mark));
            }
        });
        readouts.add(&mark_estimates);
        page.add(&readouts);

        // ---- the tray icon ----
        let tray = adw::PreferencesGroup::builder()
            .title("Tray icon")
            .description(
                "What the icon in the bar is drawn as. The previews are the tray's own pixmaps, so \
                 what is offered here is what the bar draws.",
            )
            .build();
        let style_caption = gtk4::Label::builder()
            .label("")
            .xalign(0.0)
            .wrap(true)
            .build();
        style_caption.add_css_class("caption");
        style_caption.add_css_class("dim-label");
        let styles = gtk4::Box::new(Orientation::Horizontal, 6);
        styles.set_halign(Align::Start);
        for style in TrayStyle::ALL {
            let button = self_style_button(style, commands.clone(), events.clone(), &pushed);
            styles.append(&button);
        }
        let style_row = adw::ActionRow::builder().title("Style").build();
        style_row.add_suffix(&styles);
        tray.add(&style_row);

        let colours = gtk4::Box::new(Orientation::Horizontal, 6);
        colours.set_halign(Align::Start);
        for colour in TrayTextColor::ALL {
            colours.append(&colour_button(
                colour,
                commands.clone(),
                events.clone(),
                &pushed,
            ));
        }
        let colour_row = adw::ActionRow::builder().title("Text colour").build();
        colour_row.add_suffix(&colours);
        tray.add(&colour_row);
        page.add(&tray);

        let colours_group = adw::PreferencesGroup::builder().build();
        colours_group.add(&style_caption);
        page.add(&colours_group);

        let devices_group = adw::PreferencesGroup::builder()
            .title("One icon per device")
            .description(crate::widgets::escaped(
                "Each chosen device gets an icon of its own, in the order chosen. A bar that draws \
                 only the icons it has pinned needs each id added (embr, embr-<device>).",
            ))
            .build();
        page.add(&devices_group);

        // ---- notifications ----
        let notifications_group = adw::PreferencesGroup::builder()
            .title("Notifications")
            .build();
        let notifications = adw::SwitchRow::builder()
            .title("Low battery notifications")
            .subtitle("off until you turn them on")
            .build();
        notifications.connect_active_notify({
            let commands = commands.clone();
            let pushed = pushed.clone();
            move |row| {
                let enabled = row.is_active();
                if enabled == pushed.notifications.get() {
                    return;
                }
                pushed.notifications.set(enabled);
                eprintln!(
                    "[app] battery notifications switched {}",
                    if enabled { "on" } else { "off" }
                );
                dispatch(&commands, Command::SetNotifications(enabled));
            }
        });
        notifications_group.add(&notifications);

        let thresholds = widgets::row("Low battery at");
        notifications_group.add(&thresholds);
        let snooze = widgets::row("Snooze");
        notifications_group.add(&snooze);
        page.add(&notifications_group);

        // ---- refreshing ----
        let refreshing = adw::PreferencesGroup::builder().title("Refreshing").build();
        let refresh = adw::SpinRow::with_range(5.0, 600.0, 5.0);
        refresh.set_title("Minutes… seconds between reads");
        refresh.set_subtitle("while the window is open; never faster than five seconds");
        refresh.connect_value_notify({
            let commands = commands.clone();
            let pushed = pushed.clone();
            move |row| {
                let seconds = row.value().round() as i32;
                if pushed.refresh.get() == seconds {
                    // Our own push coming back.
                    pushed.refresh.set(-1);
                    return;
                }
                pushed.refresh.set(seconds);
                eprintln!("[app] a fresh reading every {seconds}s while the window is open");
                dispatch(&commands, Command::SetRefreshSeconds(seconds.max(5) as u64));
            }
        });
        refreshing.add(&refresh);

        let watch = adw::SwitchRow::builder()
            .title("Check devices between reads")
            .subtitle("notices a device switching off within seconds")
            .build();
        watch.connect_active_notify({
            let commands = commands.clone();
            let pushed = pushed.clone();
            move |row| {
                let enabled = row.is_active();
                if enabled == pushed.watch_devices.get() {
                    return;
                }
                pushed.watch_devices.set(enabled);
                eprintln!(
                    "[app] devices will {} be checked between sweeps",
                    if enabled { "" } else { "not" }
                );
                dispatch(&commands, Command::SetWatchDevices(enabled));
            }
        });
        refreshing.add(&watch);
        page.add(&refreshing);

        // ---- the window ----
        let window_group = adw::PreferencesGroup::builder().title("Window").build();
        let close_to_tray = adw::SwitchRow::builder()
            .title("Closing the window keeps Embr running")
            .subtitle("the devices are still watched, and the tray icon stays")
            .build();
        close_to_tray.connect_active_notify({
            let commands = commands.clone();
            let pushed = pushed.clone();
            move |row| {
                let enabled = row.is_active();
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
                dispatch(&commands, Command::SetCloseToTray(enabled));
            }
        });
        window_group.add(&close_to_tray);

        let quit = widgets::row("Quit");
        let quit_button = gtk4::Button::with_label("Quit Embr");
        quit_button.set_valign(Align::Center);
        quit_button.add_css_class("destructive-action");
        quit_button.connect_clicked({
            let events = events.clone();
            move |_| dispatch_event(&events, UiEvent::Quit)
        });
        quit.add_suffix(&quit_button);
        window_group.add(&quit);
        page.add(&window_group);

        // ---- where this is written ----
        let file_group = adw::PreferencesGroup::builder()
            .title("Settings file")
            .build();
        let config = widgets::row("Written to");
        file_group.add(&config);
        page.add(&file_group);

        let dialog = adw::PreferencesDialog::builder()
            .title("Embr — Settings")
            .content_width(720)
            .content_height(860)
            .child(&page)
            .build();

        Self {
            dialog,
            notice,
            notice_group,
            styles,
            style_caption,
            colours,
            colours_group,
            devices_group,
            devices: RefCell::new(Vec::new()),
            device_rows: RefCell::new(Vec::new()),
            themes,
            theme_caption,
            show_voltage,
            level_colors,
            mark_estimates,
            notifications,
            thresholds,
            snooze,
            refresh,
            watch,
            close_to_tray,
            config,
            pushed,
            commands,
        }
    }

    /// Show the page on the window it belongs to.
    pub fn present(&self, parent: &impl IsA<gtk4::Widget>) {
        self.dialog.present(Some(parent));
    }

    /// Write one snapshot into the page: every value here comes from `core`.
    pub fn apply(&self, state: &State) {
        // The scheme is the window's own business — libadwaita asks the desktop for the rest —
        // so this is applied here, on the snapshot that carried the configuration.
        if self.pushed.theme.get() != state.theme {
            apply_theme(state.theme);
            self.show_theme(state.theme);
        }
        // Each switch is written only when it disagrees, and what we write is remembered, so the
        // snapshot that follows our own change is not mistaken for a user action.
        self.pushed.show_voltage.set(state.show_voltage);
        if self.show_voltage.is_active() != state.show_voltage {
            self.show_voltage.set_active(state.show_voltage);
        }
        self.pushed.level_colors.set(state.level_colors);
        if self.level_colors.is_active() != state.level_colors {
            self.level_colors.set_active(state.level_colors);
        }
        self.pushed.mark_estimates.set(state.mark_estimates);
        if self.mark_estimates.is_active() != state.mark_estimates {
            self.mark_estimates.set_active(state.mark_estimates);
        }

        self.notice
            .set_subtitle(state.notice.as_deref().unwrap_or(""));
        self.notice.set_visible(state.notice.is_some());
        self.notice_group.set_visible(state.notice.is_some());

        // The picker is only rebuilt when the look actually changed — otherwise every snapshot
        // would rebuild every tile to draw the same pictures.
        if self.pushed.tray_look.get() != state.tray_look {
            self.show_look(state.tray_look);
        }

        self.pushed.notifications.set(state.notifications_enabled);
        if self.notifications.is_active() != state.notifications_enabled {
            self.notifications.set_active(state.notifications_enabled);
        }
        self.thresholds.set_subtitle(&state.thresholds_text);
        self.snooze
            .set_subtitle(&snooze_caption(state.snooze_minutes));

        // A value in flight must not be dragged back by a snapshot taken before the edit; the
        // snapshot that confirms it is what closes that window.
        let seconds = i32::try_from(state.refresh_seconds).unwrap_or(60);
        let pending = self.pushed.refresh.get();
        if (pending == seconds || pending < 0) && self.refresh.value() as i32 != seconds {
            self.pushed.refresh.set(seconds);
            self.refresh.set_value(f64::from(seconds));
        }

        self.pushed.watch_devices.set(state.watch_devices);
        if self.watch.is_active() != state.watch_devices {
            self.watch.set_active(state.watch_devices);
        }
        self.pushed.close_to_tray.set(state.close_to_tray);
        if self.close_to_tray.is_active() != state.close_to_tray {
            self.close_to_tray.set_active(state.close_to_tray);
        }
        self.config.set_subtitle(&state.config_path);

        self.show_devices(&state.snapshot, &state.tray_devices);
    }

    /// Show one colour scheme as the chosen one, and say what it means.
    fn show_theme(&self, theme: Theme) {
        let mut child = self.themes.first_child();
        let mut index = 0;
        while let Some(widget) = child {
            if let Ok(button) = widget.clone().downcast::<gtk4::ToggleButton>() {
                button.set_active(Theme::ALL.get(index) == Some(&theme));
            }
            child = widget.next_sibling();
            index += 1;
        }
        self.theme_caption.set_label(theme.caption());
        self.pushed.theme.set(theme);
    }

    /// Show one look as the chosen one: the tile, the colour tiles, and the sentence under them.
    ///
    /// Public because the window applies a picker click itself: a presentation change has no
    /// business queueing behind a sweep.
    pub fn show_look(&self, look: TrayLook) {
        let mut child = self.styles.first_child();
        let mut index = 0;
        while let Some(widget) = child {
            if let Ok(button) = widget.clone().downcast::<gtk4::ToggleButton>() {
                let wanted = TrayStyle::ALL.get(index).copied();
                button.set_active(wanted == Some(look.style));
            }
            child = widget.next_sibling();
            index += 1;
        }
        // The one style with text is the percentage: nothing else reads a colour.
        self.colours_group
            .set_visible(look.style == TrayStyle::Percent);
        let mut child = self.colours.first_child();
        let mut index = 0;
        while let Some(widget) = child {
            if let Ok(button) = widget.clone().downcast::<gtk4::ToggleButton>() {
                let wanted = TrayTextColor::ALL.get(index).copied();
                button.set_active(wanted == Some(look.text_color));
            }
            child = widget.next_sibling();
            index += 1;
        }
        self.style_caption
            .set_label(&tray_style_caption(look.style));
        self.pushed.tray_look.set(look);
    }

    /// One switch per device, rebuilt only when the devices change.
    fn show_devices(&self, snapshot: &embr_core::Snapshot, chosen: &[String]) {
        let keys: Vec<String> = snapshot
            .devices
            .iter()
            .map(|device| device.key.clone())
            .collect();
        if *self.devices.borrow() != keys {
            for row in self.device_rows.borrow_mut().drain(..) {
                self.devices_group.remove(&row);
            }
            for device in &snapshot.devices {
                let row = adw::SwitchRow::builder()
                    .title(&device.name)
                    .subtitle(&device.percent_text)
                    .active(chosen.contains(&device.key))
                    .build();
                row.connect_active_notify({
                    let commands = self.commands.clone();
                    let key = device.key.clone();
                    let name = device.name.clone();
                    move |row| {
                        let on = row.is_active();
                        eprintln!(
                            "[app] {name} {} an icon of its own",
                            if on { "gets" } else { "gives up" }
                        );
                        dispatch(
                            &commands,
                            Command::SetTrayDevice {
                                key: key.clone(),
                                on,
                            },
                        );
                    }
                });
                self.devices_group.add(&row);
                self.device_rows.borrow_mut().push(row);
            }
            *self.devices.borrow_mut() = keys;
        }
    }
}

/// Tell the window, and the toolkit, which colour scheme to use.
///
/// `adw::StyleManager` is the one that knows: `Default` follows the desktop, `ForceDark`/`ForceLight`
/// override it. The desktop's accent keeps coming from the portal either way.
fn apply_theme(theme: Theme) {
    let scheme = match theme {
        Theme::System => adw::ColorScheme::Default,
        Theme::Dark => adw::ColorScheme::ForceDark,
        Theme::Light => adw::ColorScheme::ForceLight,
    };
    eprintln!(
        "[app] the window follows the {} colour scheme ({})",
        theme.label(),
        theme.as_str()
    );
    adw::StyleManager::default().set_color_scheme(scheme);
}

fn theme_chosen(theme: Theme, commands: &Sender<Command>, pushed: &Pushed) {
    if pushed.theme.get() == theme {
        return;
    }
    apply_theme(theme);
    pushed.theme.set(theme);
    dispatch(commands, Command::SetTheme(theme));
}

/// One preview tile: the pictogram itself as the picture, which is why a style cannot be offered
/// that the bar would not draw.
fn preview_image(pictogram: &embr_desktop::Pictogram, size: i32) -> gtk4::Image {
    let width = pictogram.width.max(0);
    let height = pictogram.height.max(0);
    let bytes = gtk4::glib::Bytes::from(&pictogram.data);
    let texture = gtk4::gdk::MemoryTexture::new(
        width,
        height,
        // The tray carries ARGB in network byte order, which is exactly what this format is.
        gtk4::gdk::MemoryFormat::A8r8g8b8,
        &bytes,
        width as usize * 4,
    );
    let image = gtk4::Image::from_paintable(Some(&texture));
    image.set_pixel_size(size);
    image
}

fn tile(pictogram: &embr_desktop::Pictogram, tooltip: &str) -> gtk4::ToggleButton {
    let button = gtk4::ToggleButton::new();
    button.set_child(Some(&preview_image(pictogram, 33)));
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button
}

/// The style tiles. Clicking one draws the icons immediately *and* writes the choice: a
/// presentation change has no business queueing behind a sweep.
fn self_style_button(
    style: TrayStyle,
    commands: Sender<Command>,
    events: Sender<UiEvent>,
    pushed: &Pushed,
) -> gtk4::ToggleButton {
    let button = tile(
        &embr_desktop::tray::preview(style, TrayTextColor::default()),
        style.label(),
    );
    let look = TrayLook::default();
    if style == look.style {
        button.set_active(true);
    }
    button.connect_clicked({
        let pushed = pushed.clone();
        move |button| {
            // Groups of toggles in GTK are not exclusive by themselves: the chosen one cannot be
            // switched off again, so every other tile has to be told.
            let parent = button.parent();
            unpress_siblings(&parent, button);
            let look = TrayLook {
                style,
                ..pushed.tray_look.get()
            };
            eprintln!(
                "[app] the tray icon is drawn as {} ({})",
                style.label(),
                style.as_str()
            );
            dispatch_event(&events, UiEvent::RestyleTray(look));
            dispatch(&commands, Command::SetTrayStyle(style));
        }
    });
    button
}

fn colour_button(
    colour: TrayTextColor,
    commands: Sender<Command>,
    events: Sender<UiEvent>,
    pushed: &Pushed,
) -> gtk4::ToggleButton {
    let sample = TrayLook {
        style: TrayStyle::Percent,
        text_color: colour,
    };
    let button = tile(
        &embr_desktop::tray::preview(sample.style, colour),
        colour.label(),
    );
    if colour == TrayLook::default().text_color {
        button.set_active(true);
    }
    button.connect_clicked({
        let pushed = pushed.clone();
        move |button| {
            let parent = button.parent();
            unpress_siblings(&parent, button);
            let look = TrayLook {
                text_color: colour,
                ..pushed.tray_look.get()
            };
            eprintln!(
                "[app] the tray icon's text is drawn {} ({})",
                colour.label(),
                colour.as_str()
            );
            dispatch_event(&events, UiEvent::RestyleTray(look));
            dispatch(&commands, Command::SetTrayTextColor(colour));
        }
    });
    button
}

fn unpress_siblings(parent: &Option<gtk4::Widget>, keep: &gtk4::ToggleButton) {
    let Some(parent) = parent else {
        return;
    };
    let mut child = parent.first_child();
    while let Some(widget) = child {
        if let Ok(other) = widget.clone().downcast::<gtk4::ToggleButton>()
            && &other != keep
        {
            other.set_active(false);
        }
        child = widget.next_sibling();
    }
}
