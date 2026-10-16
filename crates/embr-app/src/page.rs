//! The content pane: one page of one device.
//!
//! Two pages exist as real pages — **Battery**, which is where the readings are, and **Device**,
//! which is what the device says it is — and the rest are built when the feature they belong to
//! is built. A page that is not built yet says so, and says what it is evidence of: it is in the
//! rail because the device advertised that feature, which is a fact about the hardware rather
//! than a promise about this window.
//!
//! Nothing here decides anything. The level, the age, the alert threshold and the words for a
//! status are `core`'s (`DeviceView`, `human_age`, `thresholds_text`); this file arranges them.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::SystemTime;

use adw::prelude::*;
use gtk4::{Align, Orientation};

use embr_core::{DeviceView, FeaturePage, Snapshot, human_age, snooze_caption};

use crate::messages::{Command, State, dispatch};

/// The pane, and what it is currently showing.
pub struct Content {
    /// The container to put in the window.
    pub container: gtk4::Box,
    /// The alert level the user has just set, until a snapshot confirms it. A number the user
    /// moves must not be dragged back by a snapshot taken before their edit.
    pending: Rc<Pending>,
    title: gtk4::Label,
    subtitle: gtk4::Label,
    scrolled: gtk4::ScrolledWindow,
    shown: RefCell<Option<Shown>>,
    /// Whether the pane is currently saying there is nothing to show, so that it is not rebuilt
    /// on every snapshot for as long as that stays true.
    empty: Cell<bool>,
}

/// A number the user moved that the device thread has not confirmed yet.
#[derive(Debug, Default)]
struct Pending {
    /// The alert level the user just set, until a snapshot reports it back.
    level: RefCell<Option<u8>>,
}

impl Pending {
    fn set(&self, level: u8) {
        *self.level.borrow_mut() = Some(level);
    }

    /// Whether an edit is outstanding at all.
    fn in_flight(&self) -> bool {
        self.level.borrow().is_some()
    }

    /// Whether a snapshot may be written into the control: yes when it reports the value the user
    /// set (the round trip is done, so it is confirmed and forgotten), and yes when nothing is in
    /// flight. No while an edit is outstanding and this is some older number — that is the one
    /// that would drag the control back.
    fn confirmed(&self, level: u8) -> bool {
        if *self.level.borrow() == Some(level) {
            *self.level.borrow_mut() = None;
            return true;
        }
        !self.in_flight()
    }
}

/// What is drawn at the moment, so that a page is only rebuilt when something it prints has
/// changed — and the age row, which changes every second, is written in place instead.
struct Shown {
    key: String,
    page: FeaturePage,
    /// Everything the page prints except the ages.
    values: String,
    /// The value label of the "Updated" row: the one thing on the page that keeps moving while
    /// nothing is being read.
    updated: Option<gtk4::Label>,
}

impl Content {
    pub fn new() -> Self {
        let container = gtk4::Box::new(Orientation::Vertical, 0);

        let header = gtk4::Box::new(Orientation::Vertical, 2);
        header.set_margin_top(18);
        header.set_margin_bottom(6);
        header.set_margin_start(18);
        header.set_margin_end(18);
        let title = gtk4::Label::builder()
            .label("")
            .xalign(0.0)
            .wrap(false)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        title.add_css_class("title-3");
        let subtitle = gtk4::Label::builder().label("").xalign(0.0).build();
        subtitle.add_css_class("dim-label");
        header.append(&title);
        header.append(&subtitle);

        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vexpand(true)
            .build();

        container.append(&header);
        container.append(&scrolled);

        Self {
            container,
            pending: Rc::new(Pending::default()),
            title,
            subtitle,
            scrolled,
            shown: RefCell::new(None),
            empty: Cell::new(false),
        }
    }

    /// Draw one page of one device. Called on every snapshot; the page is only rebuilt when
    /// something it prints has changed.
    pub fn show(
        &self,
        device: Option<&DeviceView>,
        page: FeaturePage,
        state: &State,
        commands: &Sender<Command>,
        now: SystemTime,
    ) {
        let snapshot = &state.snapshot;
        let Some(device) = device else {
            // Nothing to show: no devices at all, or the device that was open has gone.
            self.title.set_label("Embr");
            self.subtitle.set_label(&empty_reason(snapshot));
            if !self.empty.replace(true) {
                self.scrolled.set_child(Some(&adw::StatusPage::builder()
                    .icon_name("dialog-warning-symbolic")
                    .title("No device to show")
                    .description(if snapshot.warnings.is_empty() {
                        "Embr found no Logitech devices. A device that is switched off, or one whose hidraw node is not readable by this user, cannot be read."
                    } else {
                        "The receivers and nodes Embr could not read are listed in the window's own log."
                    })
                    .build()));
            }
            *self.shown.borrow_mut() = None;
            return;
        };
        self.empty.set(false);

        self.title
            .set_label(&format!("{} — {}", device.name, page.label()));

        // An edit in flight freezes the page until the snapshot that confirms it arrives: the
        // one thing worse than a spin box that disagrees with the file is one that jumps back
        // under the finger that is moving it.
        if self.pending.in_flight() && !self.pending.confirmed(device.alert_below_effective) {
            return;
        }

        let values = signature(device, snapshot, &state.thresholds_text, state.show_voltage);
        let current = self.shown.borrow();
        let unchanged = current.as_ref().is_some_and(|shown| {
            shown.key == device.key && shown.page == page && shown.values == values
        });
        drop(current);
        if unchanged {
            self.refresh_ages(Some(device), now);
            return;
        }

        self.subtitle
            .set_label(&subtitle_for(device, snapshot, now));
        let (page_widget, updated) = match page {
            FeaturePage::Battery => battery_page(device, state, commands, &self.pending, now),
            FeaturePage::Device => device_page(device, snapshot),
            _ => placeholder_page(page, device),
        };
        self.set_child(&page_widget);
        *self.shown.borrow_mut() = Some(Shown {
            key: device.key.clone(),
            page,
            values,
            updated,
        });
    }

    /// Keep the age on the page and in its heading honest between sweeps.
    pub fn refresh_ages(&self, device: Option<&DeviceView>, now: SystemTime) {
        let shown = self.shown.borrow();
        let Some(shown) = shown.as_ref() else {
            return;
        };
        let Some(device) = device else {
            return;
        };
        if let Some(updated) = &shown.updated {
            updated.set_label(&age_text(device, now));
        }
    }

    /// Put a page in the pane. A `ScrolledWindow` has one child, so this replaces whatever was
    /// there — including a page that is being replaced by a redrawn one.
    fn set_child(&self, page: &adw::PreferencesPage) {
        self.scrolled.set_child(Some(page));
    }
}

/// The page that says what it is and why it is here, without pretending to be a control.
fn placeholder_page(
    page: FeaturePage,
    device: &DeviceView,
) -> (adw::PreferencesPage, Option<gtk4::Label>) {
    let group = adw::PreferencesGroup::builder().title(page.label()).build();
    let advertised = page.advertised_as().unwrap_or("what the device says it is");
    group.add(&crate::widgets::row_with(
        "Not in this window yet",
        &format!(
            "{} advertises {}, so the page is here — the controls it needs are a later step of the port.",
            device.name, advertised
        ),
    ));
    let page_widget = adw::PreferencesPage::new();
    page_widget.add(&group);
    (page_widget, None)
}

/// The battery: the reading, and the two things about this device the user can change — the level
/// it is worth being told about, and being left alone for a while.
fn battery_page(
    device: &DeviceView,
    state: &State,
    commands: &Sender<Command>,
    pending: &Rc<Pending>,
    now: SystemTime,
) -> (adw::PreferencesPage, Option<gtk4::Label>) {
    let page = adw::PreferencesPage::new();

    let battery = adw::PreferencesGroup::builder().title("Battery").build();

    let level = crate::widgets::row("Level");
    let bar = gtk4::LevelBar::new();
    bar.set_mode(gtk4::LevelBarMode::Continuous);
    bar.set_size_request(120, -1);
    bar.set_valign(Align::Center);
    bar.set_value(f64::from(device.percent.unwrap_or(0)) / 100.0);
    bar.set_visible(device.percent.is_some());
    // The same red-amber-green the rail's bar shows, from the same bands in `core`.
    crate::widgets::paint_level(&bar, device);
    let percent = gtk4::Label::builder()
        .label(&device.percent_text)
        .valign(Align::Center)
        .build();
    percent.add_css_class("numeric");
    // The one visual signal the Slint window had for a battery worth looking at, kept: the level
    // itself is red when `core` says the device needs attention (low, or reporting a fault).
    let readout = gtk4::Box::new(Orientation::Horizontal, 8);
    readout.set_valign(Align::Center);
    readout.append(&bar);
    readout.append(&percent);
    level.add_suffix(&readout);
    level.set_subtitle(if device.estimated {
        "inferred from voltage"
    } else if device.percent.is_some() {
        "the level the device reports"
    } else {
        "no level was reported"
    });
    battery.add(&level);

    let voltage = crate::widgets::row("Voltage");
    let voltage_value = gtk4::Label::builder()
        .label(&match device.millivolts {
            Some(millivolts) => format!("{millivolts} mV"),
            None => "—".to_string(),
        })
        .valign(Align::Center)
        .build();
    voltage_value.add_css_class("numeric");
    voltage.add_suffix(&voltage_value);
    // `ui.show_voltage`: a key that existed in the configuration and that nothing implemented
    // until now — which in this repository is a promise to whoever edits the file.
    voltage.set_visible(state.show_voltage);
    battery.add(&voltage);

    let status = crate::widgets::row("Status");
    status.add_suffix(
        &gtk4::Label::builder()
            .label(&device.status)
            .valign(Align::Center)
            .build(),
    );
    if device.charging {
        status.set_subtitle("on external power");
    }
    battery.add(&status);

    let updated = crate::widgets::row("Updated");
    let updated_value = gtk4::Label::builder()
        .label(age_text(device, now))
        .valign(Align::Center)
        .build();
    updated.add_suffix(&updated_value);
    battery.add(&updated);

    let alert = crate::widgets::row("Low battery alert");
    alert.set_subtitle(&if device.alert_below.is_some() {
        "set for this device".to_string()
    } else {
        format!("the shared thresholds: {}", state.thresholds_text)
    });
    // The one control here that *writes*: a device's own alert level, persisted through `core`.
    let spin = gtk4::SpinButton::with_range(1.0, 99.0, 1.0);
    spin.set_value(f64::from(device.alert_below_effective));
    spin.set_valign(Align::Center);
    spin.set_tooltip_text(Some("warn me when this device drops to this percentage"));
    spin.connect_value_changed({
        let commands = commands.clone();
        let pending = pending.clone();
        let key = device.key.clone();
        let name = device.name.clone();
        move |spin| {
            let level = spin.value().round().clamp(1.0, 99.0) as u8;
            pending.set(level);
            eprintln!("[app] {name} alerts at {level}% from now on");
            dispatch(
                &commands,
                Command::SetDeviceAlertLevel {
                    key: key.clone(),
                    level: Some(level),
                },
            );
        }
    });
    let percent = gtk4::Label::builder()
        .label("%")
        .valign(Align::Center)
        .build();
    let default = gtk4::Button::builder()
        .label("Shared")
        .valign(Align::Center)
        .tooltip_text("go back to the shared thresholds")
        .build();
    default.connect_clicked({
        let commands = commands.clone();
        let pending = pending.clone();
        let key = device.key.clone();
        move |_| {
            // What the shared thresholds will read is not known here until the device thread
            // answers, so nothing is written into the spin box: the snapshot that follows does it.
            *pending.level.borrow_mut() = None;
            eprintln!("[app] {key} goes back to the shared thresholds");
            dispatch(
                &commands,
                Command::SetDeviceAlertLevel {
                    key: key.clone(),
                    level: None,
                },
            );
        }
    });
    alert.add_suffix(&spin);
    alert.add_suffix(&percent);
    alert.add_suffix(&default);
    battery.add(&alert);

    // Snooze: the policy honours it, and this is where the user asks for it. "Until when" is the
    // configured period, said in words rather than as a number written into the window.
    let snooze = crate::widgets::row("Snooze");
    snooze.set_subtitle(&snooze_caption(state.snooze_minutes));
    let snooze_button = gtk4::Button::builder()
        .label("Be quiet")
        .valign(Align::Center)
        .build();
    snooze_button.connect_clicked({
        let commands = commands.clone();
        let key = device.key.clone();
        let name = device.name.clone();
        move |_| {
            eprintln!("[app] snooze asked for {name}");
            dispatch(&commands, Command::Snooze(key.clone()));
        }
    });
    snooze.add_suffix(&snooze_button);
    battery.add(&snooze);

    page.add(&battery);

    if let Some(error) = &device.error {
        let problem = adw::PreferencesGroup::builder().title("Problem").build();
        problem.add(&crate::widgets::row_with(
            "This device did not answer",
            error,
        ));
        page.add(&problem);
    }

    if let Some(warnings) = warnings_group(&state.snapshot) {
        page.add(&warnings);
    }

    (page, Some(updated_value))
}

/// What the device says it is, and how it is attached.
fn device_page(
    device: &DeviceView,
    snapshot: &Snapshot,
) -> (adw::PreferencesPage, Option<gtk4::Label>) {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder().title("Device").build();

    group.add(&crate::widgets::row_with("Name", &device.name));
    group.add(&crate::widgets::row_with("Kind", &device.kind));
    group.add(&crate::widgets::row_with(
        "Connected through",
        &device.connection,
    ));
    if let Some(error) = &device.error {
        group.add(&crate::widgets::row_with("Problem", error));
    }
    page.add(&group);

    if let Some(warnings) = warnings_group(snapshot) {
        page.add(&warnings);
    }
    (page, None)
}

/// What Embr could not read at all: a receiver that did not answer, a node that is not readable.
/// Not about any one device, so it belongs at the bottom of whichever page is open.
fn warnings_group(snapshot: &Snapshot) -> Option<adw::PreferencesGroup> {
    if snapshot.warnings.is_empty() {
        return None;
    }
    let group = adw::PreferencesGroup::builder()
        .title("Problems")
        .description("Things Embr could not read at all")
        .build();
    for warning in &snapshot.warnings {
        group.add(&crate::widgets::row(warning));
    }
    Some(group)
}

/// `read 10s ago · lowest of 3 devices` — the line under the page title.
fn subtitle_for(device: &DeviceView, snapshot: &Snapshot, now: SystemTime) -> String {
    let mut parts = vec![match device.captured {
        Some(captured) => format!("read {}", human_age(captured, now)),
        None => "never read".to_string(),
    }];
    if let Some(lowest) = snapshot.lowest()
        && lowest.key == device.key
        && snapshot.devices.len() > 1
    {
        parts.push(format!("lowest of {} devices", snapshot.devices.len()));
    }
    if device.estimated {
        parts.push("a value inferred from voltage, not reported by the device".to_string());
    }
    parts.join(" · ")
}

/// `never`, `just now`, `10s ago` — the age of this device's reading.
fn age_text(device: &DeviceView, now: SystemTime) -> String {
    match device.captured {
        Some(captured) => human_age(captured, now),
        None => "never".to_string(),
    }
}

fn empty_reason(snapshot: &Snapshot) -> String {
    if snapshot.devices.is_empty() {
        "nothing was found to show".to_string()
    } else {
        "the device that was open is not attached any more".to_string()
    }
}

/// Everything the current page prints, apart from the ages: a page is rebuilt when this changes
/// and not otherwise, which is what keeps a snapshot every few seconds from rebuilding widgets
/// that already say the right thing.
fn signature(
    device: &DeviceView,
    snapshot: &Snapshot,
    thresholds: &str,
    show_voltage: bool,
) -> String {
    format!(
        "{}|{:?}|{:?}|{}|{}|{:?}|{}|{:?}|{}|{thresholds}|{}|{:?}|{}",
        device.name,
        device.percent,
        device.millivolts,
        device.status,
        device.percent_text,
        device.alert_below,
        device.alert_below_effective,
        device.error,
        if device.charging { "charging" } else { "idle" },
        // Part of the signature because it decides what is drawn: toggling it must redraw.
        if show_voltage {
            "voltage"
        } else {
            "no voltage"
        },
        // Part of the signature because it decides the colour: a repaint follows a change.
        device.level,
        snapshot.warnings.join(";")
    )
}
