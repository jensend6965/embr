//! The content pane: the open device, with its pages as tabs.
//!
//! The pages a device advertises are tabs across the top (`GtkStackSwitcher` over a `GtkStack`),
//! so switching between them swaps what is *visible* instead of rebuilding
//! anything: the widgets are made once, when the device is chosen, and are then updated in place.
//! That is what keeps a reading arriving every few seconds — and a click on a device or a tab —
//! from making the window jump.
//!
//! The tabs are the device's own pages, because they come from `core::pages_for`: this cannot offer
//! a keyboard a DPI page it has no feature for. A page that is not built yet says so, and says what
//! it is evidence of.
//!
//! Nothing here decides anything. The level, the age, the alert threshold and the words for a
//! status are `core`'s (`DeviceView`, `human_age`, `thresholds_text`); this file arranges them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::SystemTime;

use adw::prelude::*;
use gtk4::{Align, Orientation};

use embr_core::{DeviceView, FeaturePage, Snapshot, human_age, snooze_caption};

use crate::messages::{Command, State, dispatch};
use crate::widgets;

/// The pane, which device and which page it is showing, and the widgets it updates in place.
pub struct Content {
    /// The container to put in the window.
    pub container: gtk4::Box,
    pending: Rc<Pending>,
    commands: Sender<Command>,
    title: gtk4::Label,
    subtitle: gtk4::Label,
    status: adw::StatusPage,
    /// What is not known about the open device's feature set, said where the tabs are.
    notice: adw::Banner,
    /// Where the tab strip and the open device's pages go.
    ///
    /// Both are replaced together, never edited: an `AdwViewSwitcher` that has shown one device's
    /// pages keeps a button for every page it has ever seen, so a strip built once and added to
    /// accumulates the pages of every device opened in the session — measured: a keyboard (6), a
    /// mouse (5) and a headset (2) left 13 pills with no room to print their names.
    tabs: gtk4::Box,
    /// The stack those tabs are driving, replaced along with them.
    ///
    /// `GtkStack`, not `AdwViewStack`: libadwaita's switcher always draws an icon slot and fills it
    /// with GTK's *missing image* box when the page's icon name is unset or unknown to this
    /// desktop's icon theme — measured here across every plausible name, only a handful resolve, so
    /// every tab came out as a warning-page glyph. GTK's own switcher has no icon slot.
    stack: RefCell<Option<gtk4::Stack>>,
    /// What Embr could not read *at all*: a receiver that did not answer, a node that is not
    /// readable. Not about one page, so it sits under the tabs whichever one is showing.
    warnings: adw::PreferencesGroup,
    warnings_for: RefCell<String>,
    shown: RefCell<Option<Shown>>,
    /// The tab the user last had open on each device, by key.
    ///
    /// The stack itself is the truth about what is showing; this exists only because a rebuild
    /// makes a *new* stack, and a new stack would otherwise put the user back on the first tab.
    /// Written from the live stack (see [`Content::remember_visible`]), never from the tick's own
    /// opinion, which is what stops a reading that arrives every five seconds from moving the user
    /// off the page they picked.
    remembered: RefCell<HashMap<String, FeaturePage>>,
    /// A page someone outside the tick asked for — the debug variable, today. Taken once, by the
    /// next build that can honour it; nothing else in the window writes the visible tab.
    requested: RefCell<Option<FeaturePage>>,
}

/// What the pane has built, and for what. Rebuilt when the device changes, when its feature set
/// arrives or changes, or when the device's own trouble changes — never because a value moved.
struct Shown {
    key: String,
    pages: Vec<FeaturePage>,
    error: Option<String>,
    parts: Vec<Part>,
}

/// One tab's live widgets.
enum Part {
    Battery(Rc<BatteryParts>),
    Device(Rc<DeviceParts>),
    Placeholder,
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

impl Content {
    pub fn new(commands: Sender<Command>) -> Self {
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

        // The notice is about the open device, and it can act: a feature set that was never read is
        // exactly what Rescan is for, so the sentence comes with the button that fixes it.
        let notice = adw::Banner::builder()
            .use_markup(false)
            .revealed(false)
            .build();
        notice.set_margin_top(6);
        notice.set_margin_bottom(6);
        notice.connect_button_clicked({
            let commands = commands.clone();
            move |_| {
                eprintln!("[app] rescan asked for from the notice");
                dispatch(&commands, Command::Rescan);
            }
        });

        let status = adw::StatusPage::builder()
            .icon_name("dialog-information-symbolic")
            .title("No device to show")
            .build();
        // Nothing is open yet, so the strip holds the status page and nothing else.
        let tabs = gtk4::Box::new(Orientation::Vertical, 0);
        tabs.append(&status);

        let warnings = adw::PreferencesGroup::builder()
            .title("Problems")
            .description("Things Embr could not read at all")
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(18)
            .visible(false)
            .build();

        container.append(&header);
        container.append(&notice);
        container.append(&tabs);
        container.append(&warnings);

        Self {
            container,
            pending: Rc::new(Pending::default()),
            commands,
            title,
            subtitle,
            status,
            notice,
            tabs,
            stack: RefCell::new(None),
            warnings,
            warnings_for: RefCell::new(String::new()),
            shown: RefCell::new(None),
            remembered: RefCell::new(HashMap::new()),
            requested: RefCell::new(None),
        }
    }

    /// The page the tabs are showing, if the stack belongs to `key`.
    ///
    /// The user can switch tabs, so the window asks rather than assumes — this is how a click on a
    /// tab gets back into the app's own state, and it is the only reading of the user's view.
    pub fn visible_page(&self, key: &str) -> Option<FeaturePage> {
        let shown = self.shown.borrow();
        if shown.as_ref().is_none_or(|shown| shown.key != key) {
            return None;
        }
        self.stack
            .borrow()
            .as_ref()
            .and_then(|stack| stack.visible_child_name())
            .and_then(|name| FeaturePage::from_id(&name))
    }

    /// Move the user to a page. The one way the view is changed from outside, and only ever for a
    /// page the open device actually has — the debug variable's entry point, not the tick's.
    pub fn request_page(&self, page: FeaturePage) {
        let shown = self.shown.borrow();
        match shown.as_ref() {
            Some(shown) if shown.pages.contains(&page) => {
                self.remembered.borrow_mut().insert(shown.key.clone(), page);
                if let Some(stack) = self.stack.borrow().as_ref() {
                    stack.set_visible_child_name(page.id());
                }
            }
            // No stack for a device with this page yet: the build that makes one will take it.
            _ => *self.requested.borrow_mut() = Some(page),
        }
    }

    /// Write down which tab is showing, so that a rebuild can put the user back on it.
    ///
    /// Read from the stack, not from a field the window keeps: a click moves the stack, so a copy
    /// the window held would be the window's opinion and not the user's.
    fn remember_visible(&self) {
        let shown = self.shown.borrow();
        let Some(shown) = shown.as_ref() else {
            return;
        };
        let visible = self
            .stack
            .borrow()
            .as_ref()
            .and_then(|stack| stack.visible_child_name())
            .and_then(|name| FeaturePage::from_id(&name));
        if let Some(page) = visible {
            self.remembered.borrow_mut().insert(shown.key.clone(), page);
        }
    }

    /// Draw the open device: its name, how old the reading is, what is not known about its feature
    /// set, and its pages as tabs.
    ///
    /// Called on every snapshot, so this is a *pull*: it writes values into widgets and never
    /// chooses anything the user chooses. Which tab is showing is the user's (read back by
    /// [`Content::visible_page`] and remembered across rebuilds); the only page it can set is one
    /// requested from outside, via [`Content::request_page`].
    pub fn sync(&self, device: Option<&DeviceView>, state: &State, now: SystemTime) {
        let snapshot = &state.snapshot;
        let Some(device) = device else {
            self.show_nothing(snapshot);
            return;
        };

        widgets::set_text(&self.title, &device.name);
        widgets::set_text(&self.subtitle, &subtitle_for(device, snapshot, now));
        self.show_notice(device);

        // Before it can be replaced: the tab that is showing is the one to come back to.
        self.remember_visible();
        let pages = device.pages.clone().unwrap_or_default();
        let stale = match self.shown.borrow().as_ref() {
            Some(shown) => {
                shown.key != device.key || shown.pages != pages || shown.error != device.error
            }
            None => true,
        };
        if stale {
            self.build(device, state, &pages, now);
        }

        let shown = self.shown.borrow();
        if let Some(shown) = shown.as_ref() {
            for part in &shown.parts {
                match part {
                    Part::Battery(parts) => parts.update(device, state, now, &self.pending),
                    Part::Device(parts) => parts.update(device),
                    Part::Placeholder => {}
                }
            }
        }
        drop(shown);
        self.show_warnings(snapshot);
    }

    /// Keep the ages honest between sweeps — the subtitle and the page's own "Updated" row are the
    /// only things that change while nothing is being read.
    pub fn refresh_ages(&self, device: Option<&DeviceView>, snapshot: &Snapshot, now: SystemTime) {
        let Some(device) = device else {
            return;
        };
        widgets::set_text(&self.subtitle, &subtitle_for(device, snapshot, now));
        let shown = self.shown.borrow();
        let Some(shown) = shown.as_ref() else {
            return;
        };
        for part in &shown.parts {
            if let Part::Battery(parts) = part {
                widgets::set_text(&parts.updated, &age_text(device, now));
            }
        }
    }

    /// Nothing to show: no devices at all, or the device that was open has gone.
    fn show_nothing(&self, snapshot: &Snapshot) {
        widgets::set_text(&self.title, "Embr");
        widgets::set_text(&self.subtitle, &empty_reason(snapshot));
        let description = if snapshot.warnings.is_empty() {
            "Embr found no Logitech devices. A device that is switched off, or one whose hidraw \
             node is not readable by this user, cannot be read."
        } else {
            "The receivers and nodes Embr could not read are listed in the window's own log."
        };
        if self.status.description().as_deref() != Some(description) {
            self.status.set_description(Some(description));
        }
        self.notice.set_revealed(false);
        // The stack goes with the device that was open, and the status page takes its place. The
        // status page is never a child of a stack, so retiring the stack cannot take it with it.
        while let Some(child) = self.tabs.first_child() {
            self.tabs.remove(&child);
        }
        self.tabs.append(&self.status);
        *self.stack.borrow_mut() = None;
        *self.shown.borrow_mut() = None;
        self.show_warnings(snapshot);
    }

    /// Say what is not known about this device's feature set, and offer the fix.
    ///
    /// Two different things look identical otherwise: "this keyboard has no lighting" and "this
    /// keyboard did not say". The tabs are only ever the pages the device named.
    fn show_notice(&self, device: &DeviceView) {
        let (title, button) = match device.pages {
            None if device.error.is_some() => (
                "Features not read — this device did not answer",
                Some("Rescan"),
            ),
            None => (
                "Features not read — asking this device what it can be asked about",
                None,
            ),
            Some(_) if device.pages_partial => (
                "This device answered for only part of its feature list, so these are the pages it named",
                None,
            ),
            Some(_) => ("", None),
        };
        let revealed = !title.is_empty();
        self.notice.set_revealed(revealed);
        if revealed {
            if self.notice.title().as_str() != title {
                self.notice.set_title(title);
            }
            if self.notice.button_label().as_deref() != button {
                self.notice.set_button_label(button);
            }
        }
    }

    /// The machine-wide problems, under the tabs. Rebuilt only when the text of them changes.
    fn show_warnings(&self, snapshot: &Snapshot) {
        let text = snapshot.warnings.join("; ");
        if *self.warnings_for.borrow() == text {
            return;
        }
        while let Some(row) = self.warnings.first_child() {
            self.warnings.remove(&row);
        }
        for warning in &snapshot.warnings {
            self.warnings.add(&widgets::row(warning));
        }
        self.warnings.set_visible(!snapshot.warnings.is_empty());
        *self.warnings_for.borrow_mut() = text;
    }

    /// Make one tab per page this device advertises, in a strip built from scratch.
    ///
    /// The strip and its stack are new every time rather than edited, because a switcher cannot be
    /// made to forget: pages taken off a stack leave their buttons behind. Called when the device
    /// changes or its feature set arrives — never because a value moved.
    fn build(&self, device: &DeviceView, state: &State, pages: &[FeaturePage], now: SystemTime) {
        let stack = gtk4::Stack::new();
        stack.set_vexpand(true);

        let mut parts = Vec::new();
        for feature in pages {
            let (page, part) = match feature {
                FeaturePage::Battery => {
                    let (page, parts) =
                        battery_page(device, state, &self.commands, &self.pending, now);
                    (page, Part::Battery(parts))
                }
                FeaturePage::Device => {
                    let (page, parts) = device_page(device);
                    (page, Part::Device(parts))
                }
                other => (placeholder_page(*other, device), Part::Placeholder),
            };
            let tab = gtk4::ScrolledWindow::builder()
                .hscrollbar_policy(gtk4::PolicyType::Never)
                .vexpand(true)
                .child(&page)
                .build();
            // The switcher takes each tab's name from its stack page's title — a page without one
            // is a blank button. No icon, deliberately: see `stack` above.
            let stack_page = stack.add_named(&tab, Some(feature.id()));
            stack_page.set_title(feature.label());
            parts.push(part);
        }

        let switcher = gtk4::StackSwitcher::builder()
            .stack(&stack)
            .halign(Align::Center)
            .margin_bottom(6)
            .build();
        // A lone page is not a choice; the strip would only be a label.
        switcher.set_visible(pages.len() > 1);

        // Which tab to land on, in the order the reasons matter: a page asked for from outside, the
        // tab this device was last on, the device's first page. Not "whatever the tick thinks" —
        // there is no such thing, and inventing one is exactly how a pull that arrives mid-click
        // moves the user off the page they chose.
        let asked = self.requested.borrow_mut().take();
        let wanted = asked
            .filter(|page| pages.contains(page))
            .or_else(|| {
                self.remembered
                    .borrow()
                    .get(&device.key)
                    .copied()
                    .filter(|page| pages.contains(page))
            })
            .or_else(|| pages.first().copied());
        if let Some(wanted) = wanted {
            stack.set_visible_child_name(wanted.id());
        }

        while let Some(child) = self.tabs.first_child() {
            self.tabs.remove(&child);
        }
        self.tabs.append(&switcher);
        self.tabs.append(&stack);
        // Kept, because the stack is the only thing that knows which tab is showing: without this
        // handle `visible_page` sees nothing, and the window reads "no page" on every tick forever.
        *self.stack.borrow_mut() = Some(stack);
        *self.shown.borrow_mut() = Some(Shown {
            key: device.key.clone(),
            pages: pages.to_vec(),
            error: device.error.clone(),
            parts,
        });
    }
}

/// The battery tab: the reading, and the two things about this device the user can change — the
/// level it is worth being told about, and being left alone for a while.
struct BatteryParts {
    level: adw::ActionRow,
    bar: gtk4::LevelBar,
    percent: gtk4::Label,
    voltage: adw::ActionRow,
    voltage_value: gtk4::Label,
    status: adw::ActionRow,
    status_value: gtk4::Label,
    updated: gtk4::Label,
    alert: adw::ActionRow,
    spin: gtk4::SpinButton,
    snooze: adw::ActionRow,
}

impl BatteryParts {
    fn update(&self, device: &DeviceView, state: &State, now: SystemTime, pending: &Pending) {
        widgets::set_text(&self.percent, &device.percent_text);
        self.bar
            .set_value(f64::from(device.percent.unwrap_or(0)) / 100.0);
        self.bar.set_visible(device.percent.is_some());
        widgets::paint_level(&self.bar, device);
        widgets::set_subtitle(
            &self.level,
            if device.estimated {
                "inferred from voltage"
            } else if device.percent.is_some() {
                "the level the device reports"
            } else {
                "no level was reported"
            },
        );
        widgets::set_text(
            &self.voltage_value,
            &match device.millivolts {
                Some(millivolts) => format!("{millivolts} mV"),
                None => "—".to_string(),
            },
        );
        self.voltage.set_visible(state.show_voltage);
        widgets::set_text(&self.status_value, &device.status);
        widgets::set_subtitle(
            &self.status,
            if device.charging {
                "on external power"
            } else {
                ""
            },
        );
        widgets::set_text(&self.updated, &age_text(device, now));
        widgets::set_subtitle(
            &self.alert,
            &if device.alert_below.is_some() {
                "set for this device".to_string()
            } else {
                format!("the shared thresholds: {}", state.thresholds_text)
            },
        );
        // A spin box the user just moved is left alone until the file agrees: a snapshot taken
        // before their edit is the one thing that would drag it back under their finger.
        if pending.confirmed(device.alert_below_effective) {
            let wanted = f64::from(device.alert_below_effective);
            if (self.spin.value() - wanted).abs() > 0.5 {
                self.spin.set_value(wanted);
            }
        }
        widgets::set_subtitle(&self.snooze, &snooze_caption(state.snooze_minutes));
    }
}

/// Build the battery tab, and the handles its values are written through afterwards.
fn battery_page(
    device: &DeviceView,
    state: &State,
    commands: &Sender<Command>,
    pending: &Rc<Pending>,
    now: SystemTime,
) -> (adw::PreferencesPage, Rc<BatteryParts>) {
    let page = adw::PreferencesPage::new();

    let battery = adw::PreferencesGroup::builder().title("Battery").build();

    let level = widgets::row("Level");
    let bar = gtk4::LevelBar::new();
    bar.set_mode(gtk4::LevelBarMode::Continuous);
    bar.set_size_request(120, -1);
    bar.set_valign(Align::Center);
    bar.set_value(f64::from(device.percent.unwrap_or(0)) / 100.0);
    bar.set_visible(device.percent.is_some());
    // The same red-amber-green the rail's bar shows, from the same bands in `core`.
    widgets::paint_level(&bar, device);
    let percent = gtk4::Label::builder()
        .label(&device.percent_text)
        .valign(Align::Center)
        .build();
    percent.add_css_class("numeric");
    let readout = gtk4::Box::new(Orientation::Horizontal, 8);
    readout.set_valign(Align::Center);
    readout.append(&bar);
    readout.append(&percent);
    level.add_suffix(&readout);
    battery.add(&level);

    let voltage = widgets::row("Voltage");
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

    let status = widgets::row("Status");
    let status_value = gtk4::Label::builder()
        .label(&device.status)
        .valign(Align::Center)
        .build();
    status.add_suffix(&status_value);
    battery.add(&status);

    let updated = widgets::row("Updated");
    let updated_value = gtk4::Label::builder()
        .label(age_text(device, now))
        .valign(Align::Center)
        .build();
    updated.add_suffix(&updated_value);
    battery.add(&updated);

    let alert = widgets::row("Low battery alert");
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
    let percent_sign = gtk4::Label::builder()
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
    alert.add_suffix(&percent_sign);
    alert.add_suffix(&default);
    battery.add(&alert);

    // Snooze: the policy honours it, and this is where the user asks for it. "Until when" is the
    // configured period, said in words rather than as a number written into the window.
    let snooze = widgets::row("Snooze");
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

    let parts = Rc::new(BatteryParts {
        level,
        bar,
        percent,
        voltage,
        voltage_value,
        status,
        status_value,
        updated: updated_value,
        alert,
        spin,
        snooze,
    });
    parts.update(device, state, now, pending);
    (page, parts)
}

/// What the device says it is, and how it is attached.
struct DeviceParts {
    name: adw::ActionRow,
    kind: adw::ActionRow,
    connection: adw::ActionRow,
}

impl DeviceParts {
    fn update(&self, device: &DeviceView) {
        widgets::set_subtitle(&self.name, &device.name);
        widgets::set_subtitle(&self.kind, &device.kind_label);
        widgets::set_subtitle(&self.connection, &device.connection);
    }
}

fn device_page(device: &DeviceView) -> (adw::PreferencesPage, Rc<DeviceParts>) {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder().title("Device").build();

    let name = widgets::row_with("Name", &device.name);
    let kind = widgets::row_with("Kind", &device.kind_label);
    let connection = widgets::row_with("Connected through", &device.connection);
    group.add(&name);
    group.add(&kind);
    group.add(&connection);
    if let Some(error) = &device.error {
        group.add(&widgets::row_with("Problem", error));
    }
    page.add(&group);

    let parts = Rc::new(DeviceParts {
        name,
        kind,
        connection,
    });
    (page, parts)
}

/// The page that says what it is and why it is here, without pretending to be a control.
fn placeholder_page(page: FeaturePage, device: &DeviceView) -> adw::PreferencesPage {
    // Escaped: a preference group parses its title as Pango markup, and `DPI & sensitivity` is
    // exactly the string that breaks it — the title vanished from the page and GTK warned
    // `Entity did not end with a semicolon` (found by clicking the row for real).
    let group = adw::PreferencesGroup::builder()
        .title(widgets::escaped(page.label()))
        .build();
    let advertised = page.advertised_as().unwrap_or("what the device says it is");
    group.add(&widgets::row_with(
        "Not in this window yet",
        &format!(
            "{} advertises {}, so the page is here — the controls it needs are a later step of the port.",
            device.name, advertised
        ),
    ));
    let page_widget = adw::PreferencesPage::new();
    page_widget.add(&group);
    page_widget
}

/// `read 10s ago · lowest of 3 devices` — the line under the device's name.
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
