//! Embr, GTK 4 + libadwaita prototype — not the app, just the shape of it.
//!
//! What this is for: showing what the *same* design looks like in real GTK widgets, whether Rust
//! can drive them, and what the desktop theme costs us. It draws the devices page only, with the
//! numbers hardcoded — no HID++, no config, no tray.
//!
//! The one live behaviour is the appearance: the app follows the desktop's colour scheme, and
//! six seconds in it flips itself to light so the change can be photographed without touching the
//! user's desktop settings.

use adw::prelude::*;
use gtk4::{glib, Align, Orientation};
use std::time::Instant;

/// Set to turn the automatic dark -> light flip off (`EMBR_SPIKE_NO_FLIP=1`).
fn flip_enabled() -> bool {
    std::env::var_os("EMBR_SPIKE_NO_FLIP").is_none()
}

fn main() -> glib::ExitCode {
    let started = Instant::now();
    let app = adw::Application::builder()
        .application_id("io.github.jensend6965.EmbrSpike")
        .build();
    app.connect_activate(move |app| build(app, started));
    app.run()
}

fn build(app: &adw::Application, started: Instant) {
    let style = adw::StyleManager::default();
    eprintln!(
        "[spike] style manager: color-scheme={:?} dark={} system-supports-color-schemes={}",
        style.color_scheme(),
        style.is_dark(),
        style.system_supports_color_schemes(),
    );
    // What libadwaita thinks the desktop's accent is — the value every GTK widget then uses.
    let accent = style.accent_color();
    let rgba = accent.to_rgba();
    eprintln!(
        "[spike] accent: {:?} -> #{:02X}{:02X}{:02X}",
        accent,
        (rgba.red() * 255.0).round() as u8,
        (rgba.green() * 255.0).round() as u8,
        (rgba.blue() * 255.0).round() as u8,
    );
    style.connect_dark_notify(|m| {
        eprintln!(
            "[spike] scheme changed -> color-scheme={:?} dark={}",
            m.color_scheme(),
            m.is_dark()
        );
    });

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Embr — GTK spike")
        .default_width(1260)
        .default_height(900)
        .build();

    // ---- header ----
    let header = gtk4::HeaderBar::new();
    let rescan = gtk4::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Rescan")
        .build();
    rescan.add_css_class("suggested-action");
    let gear = gtk4::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Settings")
        .build();
    header.pack_start(&rescan);
    header.pack_end(&gear);
    header.set_title_widget(Some(&gtk4::Label::new(Some("Embr"))));

    // ---- rail: the devices, and the selected one's features under it ----
    let list = gtk4::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk4::SelectionMode::Single);
    list.set_margin_top(12);
    list.set_margin_bottom(12);
    list.set_margin_start(12);
    list.set_margin_end(12);

    let g703 = adw::ExpanderRow::builder()
        .title("G703 LS")
        .subtitle("Discharging · Mouse · 10s ago")
        .expanded(true)
        .build();
    g703.add_suffix(&percent("~56%", true));
    for feature in [
        "Battery",
        "DPI &amp; sensitivity",
        "Lighting",
        "Buttons",
        "Profiles",
        "Device",
    ] {
        let row = adw::ActionRow::builder().title(feature).build();
        if feature == "DPI & sensitivity" {
            row.add_prefix(&gtk4::Image::from_icon_name("go-next-symbolic"));
        }
        g703.add_row(&row);
    }
    list.append(&g703);

    for (name, sub, pct) in [
        ("G913 TKL", "Discharging · Keyboard · 10s ago", "~86%"),
        ("G733 Headset", "Not answering · USB · 4s ago", "—"),
    ] {
        let row = adw::ActionRow::builder().title(name).subtitle(sub).build();
        row.add_suffix(&percent(pct, pct != "—"));
        list.append(&row);
    }

    let rail_scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&list)
        .vexpand(true)
        .build();

    let rail = gtk4::Box::new(Orientation::Vertical, 0);
    rail.set_size_request(360, -1);
    let rail_title = gtk4::Label::builder()
        .label("DEVICES")
        .xalign(0.0)
        .margin_top(14)
        .margin_start(18)
        .build();
    rail_title.add_css_class("caption-heading");
    rail.append(&rail_title);
    rail.append(&rail_scroll);

    // ---- the feature page: cards of rows, the shape the sheet proposed ----
    let page = adw::PreferencesPage::new();

    let sensitivity = adw::PreferencesGroup::builder()
        .title("Sensitivity")
        .description("Four slots, the shape G HUB uses: a slider each, and which one the button cycle steps through.")
        .build();
    for (slot, dpi, on) in [
        ("Slot 1", 800.0, true),
        ("Slot 2", 1600.0, true),
        ("Slot 3", 3200.0, true),
        ("Slot 4", 200.0, false),
    ] {
        let row = adw::ActionRow::builder()
            .title(slot)
            .subtitle(format!("{} dpi", dpi as i32))
            .build();
        let scale = gtk4::Scale::with_range(Orientation::Horizontal, 200.0, 6400.0, 50.0);
        scale.set_value(dpi);
        scale.set_draw_value(false);
        scale.set_size_request(240, -1);
        scale.set_valign(Align::Center);
        row.add_suffix(&scale);
        let sw = gtk4::Switch::builder().active(on).valign(Align::Center).build();
        row.add_suffix(&sw);
        sensitivity.add(&row);
    }
    page.add(&sensitivity);

    let pointer = adw::PreferencesGroup::builder().title("Pointer").build();
    let rate_row = adw::ActionRow::builder()
        .title("Report rate")
        .subtitle("how often the mouse reports")
        .build();
    let seg = gtk4::Box::new(Orientation::Horizontal, 0);
    seg.add_css_class("linked");
    seg.set_valign(Align::Center);
    let mut first: Option<gtk4::ToggleButton> = None;
    for (label, on) in [
        ("125", false),
        ("250", false),
        ("500", false),
        ("1000 Hz", true),
    ] {
        let button = gtk4::ToggleButton::with_label(label);
        if let Some(f) = &first {
            button.set_group(Some(f));
        } else {
            first = Some(button.clone());
        }
        button.set_active(on);
        seg.append(&button);
    }
    rate_row.add_suffix(&seg);
    pointer.add(&rate_row);

    let speed = adw::ActionRow::builder()
        .title("Pointer speed")
        .subtitle("applies to the host, not the device")
        .build();
    let speed_scale = gtk4::Scale::with_range(Orientation::Horizontal, -10.0, 10.0, 1.0);
    speed_scale.set_draw_value(false);
    speed_scale.set_size_request(240, -1);
    speed_scale.set_valign(Align::Center);
    speed.add_suffix(&speed_scale);
    speed.add_suffix(&gtk4::Label::new(Some("± 0")));
    pointer.add(&speed);

    pointer.add(
        &adw::SwitchRow::builder()
            .title("Written to the device as you move it")
            .active(true)
            .build(),
    );
    page.add(&pointer);

    let lighting = adw::PreferencesGroup::builder().title("Lighting").build();
    let mode = adw::ComboRow::builder().title("Mode").build();
    mode.set_model(Some(&gtk4::StringList::new(&[
        "Off",
        "Fixed",
        "Cycle",
        "Breathing",
    ])));
    mode.set_selected(1);
    lighting.add(&mode);
    let colour = adw::ActionRow::builder()
        .title("Colour")
        .subtitle("#BD93F9 · all zones")
        .build();
    let swatch = gtk4::DrawingArea::new();
    swatch.set_size_request(28, 18);
    swatch.set_valign(Align::Center);
    swatch.set_draw_func(|_, cr, w, h| {
        cr.set_source_rgb(0.741, 0.576, 0.976);
        cr.rectangle(0.0, 0.0, w as f64, h as f64);
        let _ = cr.fill();
    });
    colour.add_suffix(&swatch);
    colour.add_suffix(&gtk4::Button::with_label("All zones…"));
    lighting.add(&colour);
    page.add(&lighting);

    // ---- appearance: what the desktop says, and how little it costs to follow it ----
    let appearance = adw::PreferencesGroup::builder()
        .title("Appearance")
        .description("libadwaita follows the desktop's colour scheme by itself; this row is the whole of the code.")
        .build();
    let appearance_row = adw::ActionRow::builder()
        .title("Theme")
        .subtitle("follow the desktop")
        .build();
    let appearance_seg = gtk4::Box::new(Orientation::Horizontal, 0);
    appearance_seg.add_css_class("linked");
    appearance_seg.set_valign(Align::Center);
    let mut first: Option<gtk4::ToggleButton> = None;
    for (label, scheme) in [
        ("System", adw::ColorScheme::Default),
        ("Dark", adw::ColorScheme::ForceDark),
        ("Light", adw::ColorScheme::ForceLight),
    ] {
        let button = gtk4::ToggleButton::with_label(label);
        if let Some(f) = &first {
            button.set_group(Some(f));
        } else {
            first = Some(button.clone());
        }
        button.set_active(matches!(scheme, adw::ColorScheme::Default));
        let manager = style.clone();
        button.connect_clicked(move |_| manager.set_color_scheme(scheme));
        appearance_seg.append(&button);
    }
    appearance_row.add_suffix(&appearance_seg);
    appearance.add(&appearance_row);
    page.add(&appearance);

    // ---- the roadmap's controls: what a GTK 4 app gets for free when these features land ----
    let lighting2 = adw::PreferencesGroup::builder()
        .title("Keyboard lighting (roadmap)")
        .description("A colour picker, a brightness number and a zone map: two of them are stock widgets here.")
        .build();
    let colour_row = adw::ActionRow::builder()
        .title("Colour")
        .subtitle("all zones · written as you pick")
        .build();
    let dialog = gtk4::ColorDialog::new();
    let colour_button = gtk4::ColorDialogButton::new(Some(dialog));
    colour_button.set_rgba(&gtk4::gdk::RGBA::new(0.741, 0.576, 0.976, 1.0));
    colour_button.set_valign(Align::Center);
    colour_row.add_suffix(&colour_button);
    lighting2.add(&colour_row);

    let bright = adw::SpinRow::with_range(0.0, 100.0, 5.0);
    bright.set_title("Brightness");
    bright.set_subtitle("percent");
    bright.set_value(80.0);
    lighting2.add(&bright);

    let zones_row = adw::ActionRow::builder()
        .title("Zones")
        .subtitle("drawn on a GtkDrawingArea — the one part that is custom work in any toolkit")
        .build();
    let zones = gtk4::DrawingArea::new();
    zones.set_content_width(180);
    zones.set_content_height(56);
    zones.set_valign(Align::Center);
    zones.set_draw_func(|_, cr, w, h| {
        // five key zones; the third one is "selected", drawn in the accent colour
        let (w, h) = (f64::from(w), f64::from(h));
        let accent = [0.741f64, 0.576, 0.976];
        for i in 0..5 {
            let x = 4.0 + i as f64 * (w - 8.0) / 5.0;
            let zw = (w - 8.0) / 5.0 - 4.0;
            if i == 2 {
                cr.set_source_rgb(accent[0], accent[1], accent[2]);
            } else {
                cr.set_source_rgba(0.6, 0.6, 0.65, 0.35);
            }
            cr.rectangle(x, 6.0, zw, h - 12.0);
            let _ = cr.fill();
        }
    });
    zones_row.add_suffix(&zones);
    lighting2.add(&zones_row);
    page.add(&lighting2);

    let buttons = adw::PreferencesGroup::builder()
        .title("Buttons (roadmap)")
        .description("Key capture, an editable row, and a list you can add to — all stock.")
        .build();
    let capture_row = adw::ActionRow::builder()
        .title("Assign a key")
        .subtitle("press one with this window focused")
        .build();
    let capture = gtk4::Button::with_label("press a key…");
    capture.set_valign(Align::Center);
    let keys = gtk4::EventControllerKey::new();
    let label = capture.clone();
    keys.connect_key_pressed(move |_, keyval, _, _| {
        label.set_label(&keyval.name().map(|n| n.to_string()).unwrap_or_else(|| format!("{keyval:?}")));
        glib::Propagation::Stop
    });
    capture.add_controller(keys);
    capture_row.add_suffix(&capture);
    buttons.add(&capture_row);

    let profile = adw::EntryRow::builder()
        .title("Profile name")
        .text("Desktop")
        .build();
    buttons.add(&profile);

    let list = gtk4::ListBox::new();
    list.add_css_class("boxed-list");
    for name in ["Desktop", "Shooter"] {
        list.append(
            &adw::ActionRow::builder()
                .title(name)
                .subtitle("on the mouse, slot 1")
                .build(),
        );
    }
    let list_row = adw::ActionRow::builder().title("Onboard profiles").build();
    list_row.add_suffix(&list);
    buttons.add(&list_row);

    let add_row = adw::ActionRow::builder().title("Add / remove a profile").build();
    let add = gtk4::Button::with_label("Add");
    let remove = gtk4::Button::with_label("Remove");
    let toast_overlay = adw::ToastOverlay::new();
    let added = list.clone();
    let toast = toast_overlay.clone();
    add.connect_clicked(move |_| {
        let n = added.observe_children().n_items();
        added.append(
            &adw::ActionRow::builder()
                .title(format!("Profile {}", n + 1))
                .subtitle("written to the device")
                .build(),
        );
        toast.add_toast(adw::Toast::new("Profile written to the device"));
    });
    let removed = list.clone();
    remove.connect_clicked(move |_| {
        if let Some(last) = removed.observe_children().item(removed.observe_children().n_items() - 1)
            && let Ok(row) = last.downcast::<gtk4::Widget>()
        {
            removed.remove(&row);
        }
    });
    add_row.add_suffix(&add);
    add_row.add_suffix(&remove);
    buttons.add(&add_row);
    page.add(&buttons);

    let page_scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&page)
        .vexpand(true)
        .build();

    toast_overlay.set_child(Some(&page_scroll));
    let body = gtk4::Box::new(Orientation::Horizontal, 0);
    body.append(&rail);
    body.append(&gtk4::Separator::new(Orientation::Vertical));
    body.append(&toast_overlay);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&body));
    window.set_content(Some(&toolbar));

    window.present();
    eprintln!(
        "[spike] window built and presented after {} ms",
        started.elapsed().as_millis()
    );

    {
        let w = window.clone();
        let m = style.clone();
        let t0 = started;
        glib::timeout_add_seconds_local(1, move || {
            eprintln!(
                "[spike] t+{}s is_dark={} window css classes: {:?}",
                t0.elapsed().as_secs(),
                m.is_dark(),
                w.css_classes().iter().map(|c| c.to_string()).collect::<Vec<_>>()
            );
            glib::ControlFlow::Continue
        });
    }

    // ---- theme experiments: which route actually restyles the window? ----
    // EMBR_SPIKE_AT_STARTUP=libadwaita|gtk-theme   — before the first map
    // EMBR_SPIKE_AT_4S=libadwaita|gtk-theme        — four seconds in, the real "user changed it" case
    match std::env::var("EMBR_SPIKE_AT_STARTUP").as_deref() {
        Ok("libadwaita") => style.set_color_scheme(adw::ColorScheme::ForceLight),
        Ok("gtk-theme") => gtk_prefer_dark(false),
        _ => {}
    }

    {
        let w = window.clone();
        let m = style.clone();
        let t0 = started;
        glib::timeout_add_seconds_local(1, move || {
            eprintln!(
                "[spike] t+{}s is_dark={} theme={:?} window css classes: {:?}",
                t0.elapsed().as_secs(),
                m.is_dark(),
                gtk_theme_name(),
                w.css_classes().iter().map(|c| c.to_string()).collect::<Vec<_>>()
            );
            glib::ControlFlow::Continue
        });
    }

    if let Ok(route) = std::env::var("EMBR_SPIKE_AT_4S") {
        let manager = style.clone();
        glib::timeout_add_seconds_local_once(4, move || {
            eprintln!("[spike] switching the scheme from inside the app via {route}");
            match route.as_str() {
                "gtk-theme" => gtk_prefer_dark(false),
                _ => manager.set_color_scheme(adw::ColorScheme::ForceLight),
            }
        });
    }

    // Drive the list by itself three seconds in, so a screenshot can show the row it adds and the
    // toast that says it was written — nothing on this desktop can inject a click.
    {
        let row = add.clone();
        glib::timeout_add_seconds_local_once(3, move || row.emit_clicked());
        // and scroll the page down, because the roadmap groups are the ones worth photographing
        let scroll = page_scroll.clone();
        glib::timeout_add_seconds_local(1, move || {
            let adj = scroll.vadjustment();
            let bottom = adj.upper() - adj.page_size();
            if bottom > 0.0 {
                adj.set_value(bottom);
                eprintln!("[spike] scrolled to {bottom:.0} (page {:.0}, viewport {:.0})", adj.upper(), adj.page_size());
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    // Open the stock colour picker, so a screenshot can show the one control the roadmap needs
    // most and that Slint has no answer for.
    if std::env::var_os("EMBR_SPIKE_COLOUR").is_some() {
        let button = colour_button.clone();
        glib::timeout_add_seconds_local_once(4, move || {
            eprintln!("[spike] opening the colour dialog");
            button.activate();
        });
    }

    // The photograph runs early: flip to light from the inside so a scheme change can be seen
    // without touching the desktop's own setting.
    if flip_enabled() {
        let manager = style.clone();
        glib::timeout_add_seconds_local_once(6, move || {
            eprintln!("[spike] flipping the scheme from inside the app");
            manager.set_color_scheme(adw::ColorScheme::ForceLight);
        });
    }
}

/// The GTK-side switch: what `settings.ini`'s `gtk-application-prefer-dark-theme` sets.
fn gtk_prefer_dark(dark: bool) {
    if let Some(settings) = gtk4::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(dark);
    }
}

fn gtk_theme_name() -> String {
    gtk4::Settings::default()
        .and_then(|s| s.gtk_theme_name())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "?".into())
}

fn percent(text: &str, bar: bool) -> gtk4::Box {
    let row = gtk4::Box::new(Orientation::Horizontal, 8);
    row.set_valign(Align::Center);
    if bar {
        let level = gtk4::LevelBar::new();
        level.set_mode(gtk4::LevelBarMode::Continuous);
        level.set_size_request(90, -1);
        level.set_valign(Align::Center);
        level.set_value(if text.starts_with("~5") { 0.56 } else { 0.86 });
        row.append(&level);
    }
    let label = gtk4::Label::new(Some(text));
    label.add_css_class("numeric");
    row.append(&label);
    row
}
