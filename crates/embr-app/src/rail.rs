//! The device rail: one row per device, and nothing under it.
//!
//! This used to be a card per device with that device's pages folded out beneath it. Two things
//! were wrong with that shape: choosing a device rearranged the rail and rebuilt the pane beside
//! it, and the text under a card (a long device name, or the sentence a device gets when it does
//! not answer) decided how wide the whole column was. The pages are tabs in the pane now, and this
//! column holds one width — see [`WIDTH`].
//!
//! Rows are `AdwActionRow`s in a `GtkListBox`. That is not a style choice: a row only reports a
//! click when its parent is a list box *and* the row is activatable, which is why every row here
//! says so explicitly.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::SystemTime;

use adw::prelude::*;
use gtk4::{Align, Orientation};

use embr_core::{DeviceView, Snapshot, human_age, summary_text};

use crate::messages::UiEvent;
use crate::widgets;

/// How wide the rail is before anyone drags it: wide enough for a device name and its percentage,
/// narrow enough that the pages keep the room. The redesign sheet draws the same 340 px at 1240 px.
///
/// `main` puts this column in a `GtkPaned` as the start child, with `resize_start_child(false)`, so
/// the width is this or whatever the user dragged it to — and never a side effect of what a device
/// is called. A wrapping title cannot widen it: the pane's position is the width.
pub const WIDTH: i32 = 450;

pub struct Rail {
    /// The container to put in the window.
    pub container: gtk4::Widget,
    list: gtk4::ListBox,
    /// What the column says above the rows: the devices it has and the worst reading among them.
    ///
    /// This is the sentence the window's *header* used to carry (`3 devices · lowest ~47%`), moved
    /// to sit over the list it is about — the header's line is the app's own name now — and written
    /// the way this column already writes its heading.
    heading: gtk4::Label,
    scrub: gtk4::Label,
    empty: gtk4::Label,
    rows: Rc<RefCell<Vec<Row>>>,
}

struct Row {
    key: String,
    row: adw::ActionRow,
    bar: gtk4::LevelBar,
    percent: gtk4::Label,
}

impl Rail {
    pub fn new(events: Sender<UiEvent>) -> Self {
        let header = gtk4::Box::new(Orientation::Horizontal, 8);
        header.set_margin_top(14);
        header.set_margin_bottom(6);
        header.set_margin_start(18);
        header.set_margin_end(18);
        let heading = gtk4::Label::builder().label("DEVICES").xalign(0.0).build();
        heading.add_css_class("caption-heading");
        let scrub = gtk4::Label::builder()
            .label("")
            .xalign(1.0)
            .hexpand(true)
            .build();
        scrub.add_css_class("caption");
        scrub.add_css_class("dim-label");
        header.append(&heading);
        header.append(&scrub);

        // One list box for every device rather than one per device: the selection becomes a
        // property of the column instead of something each card has to be told four times a
        // second, and the theme draws the open device's row selected for us.
        let list = gtk4::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk4::SelectionMode::Single);
        list.set_valign(Align::Start);
        list.set_margin_top(6);
        list.set_margin_bottom(18);
        list.set_margin_start(12);
        list.set_margin_end(12);

        let rows: Rc<RefCell<Vec<Row>>> = Rc::new(RefCell::new(Vec::new()));
        list.connect_row_activated({
            let rows = rows.clone();
            let events = events.clone();
            move |_, row| {
                // A row's index is the device's index: `rebuild` keeps the two lists in step.
                let index = row.index();
                if index < 0 {
                    return;
                }
                if let Some(entry) = rows.borrow().get(index as usize) {
                    let _ = events.send(UiEvent::Select(entry.key.clone()));
                }
            }
        });

        // Shown instead of the list when nothing was found. A sentence rather than an empty box,
        // because an empty box reads as "still loading".
        let empty = gtk4::Label::builder()
            .label("No Logitech devices found")
            .wrap(true)
            .xalign(0.0)
            .margin_top(12)
            .margin_start(18)
            .margin_end(18)
            .build();
        empty.add_css_class("dim-label");

        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .child(&list)
            .vexpand(true)
            .build();

        let container = gtk4::Box::new(Orientation::Vertical, 0);
        container.append(&header);
        container.append(&scrolled);
        container.append(&empty);
        container.set_size_request(WIDTH, -1);

        Self {
            container: container.upcast(),
            list,
            heading,
            scrub,
            empty,
            rows,
        }
    }

    /// Draw the devices, with the open one selected.
    ///
    /// Values are written in place, and only when they changed: this runs four times a second, so
    /// nothing here may rebuild a widget or write a label that already says the same thing.
    pub fn sync(&self, snapshot: &Snapshot, selected: Option<&str>, now: SystemTime) {
        let wanted: Vec<String> = snapshot
            .devices
            .iter()
            .map(|device| device.key.clone())
            .collect();
        let current: Vec<String> = self
            .rows
            .borrow()
            .iter()
            .map(|entry| entry.key.clone())
            .collect();
        // Rebuilt only when the set or the order changed: a watch pass publishes every few seconds
        // and the order only moves when a battery crosses another, so this is not what the user
        // sees while clicking.
        if wanted != current {
            self.rebuild(&snapshot.devices);
        }

        let rows = self.rows.borrow();
        for (entry, device) in rows.iter().zip(snapshot.devices.iter()) {
            widgets::set_title(&entry.row, &device.name);
            let summary = summary(device, now);
            if entry.row.subtitle().as_deref() != Some(summary.as_str()) {
                entry.row.set_subtitle(&summary);
            }
            if entry.percent.text() != device.percent_text {
                entry.percent.set_label(&device.percent_text);
            }
            entry
                .bar
                .set_value(f64::from(device.percent.unwrap_or(0)) / 100.0);
            entry.bar.set_visible(device.percent.is_some());
            // The bar carries the level's colour; the percentage beside it stays ordinary text.
            widgets::paint_level(&entry.bar, device);
        }

        // The open device is the selected row — but only when it is not already, so a click the
        // list box is in the middle of handling is left alone. Compared by index: a `ListBoxRow`
        // and an `AdwActionRow` are different types and only their position says which is which.
        let wanted_index = snapshot
            .devices
            .iter()
            .position(|device| Some(device.key.as_str()) == selected);
        let selected_index = self.list.selected_row().map(|row| row.index());
        if wanted_index.map(|index| index as i32) != selected_index {
            let row = wanted_index
                .and_then(|index| rows.get(index))
                .map(|entry| entry.row.clone());
            self.list.select_row(row.as_ref());
        }
        drop(rows);

        // What the column holds, over the rows it holds it for. Uppercased because that is how this
        // heading has always been written; the sentence itself is `core`'s.
        let heading = summary_text(snapshot).to_uppercase();
        if self.heading.text() != heading {
            self.heading.set_label(&heading);
        }
        let scrub = taken_label(snapshot, now);
        if self.scrub.text() != scrub {
            self.scrub.set_label(&scrub);
        }
        self.empty.set_visible(snapshot.devices.is_empty());
        self.list.set_visible(!snapshot.devices.is_empty());
    }

    /// Keep the ages honest between sweeps: the subtitles and the scrub time are the only things
    /// here that change while nothing is being read.
    pub fn refresh_ages(&self, snapshot: &Snapshot, now: SystemTime) {
        let scrub = taken_label(snapshot, now);
        if self.scrub.text() != scrub {
            self.scrub.set_label(&scrub);
        }
        for (entry, device) in self.rows.borrow().iter().zip(snapshot.devices.iter()) {
            let summary = summary(device, now);
            if entry.row.subtitle().as_deref() != Some(summary.as_str()) {
                entry.row.set_subtitle(&summary);
            }
        }
    }

    /// Whether the rail is still showing this device — a device that left while it was open.
    pub fn has(&self, key: &str) -> bool {
        self.rows.borrow().iter().any(|entry| entry.key == *key)
    }

    fn rebuild(&self, devices: &[DeviceView]) {
        self.rows.borrow_mut().clear();
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for device in devices {
            self.add(device);
        }
    }

    /// What each row is actually laid out as — for `EMBR_DEBUG_HEADLESS`, which checks the rail
    /// without putting a window on screen: one glyph, one line of name, and a height that every row
    /// shares.
    pub fn probe(&self) -> Vec<String> {
        self.rows
            .borrow()
            .iter()
            .map(|entry| {
                let row: gtk4::Widget = entry.row.clone().upcast();
                let (_, natural, _, _) = row.measure(gtk4::Orientation::Vertical, WIDTH);
                format!(
                    "{}: title_lines={} glyphs={} height={natural}",
                    entry.key,
                    entry.row.title_lines(),
                    count_type(&row, "GtkDrawingArea"),
                )
            })
            .collect()
    }

    fn add(&self, device: &DeviceView) {
        let row = widgets::row_with(&device.name, &summary(device, SystemTime::now()));
        // **A `GtkListBoxRow` is not activatable unless it is told so.** The property defaults to
        // `false` in GTK 4, and libadwaita does not turn it on for an `AdwActionRow`, so a click on
        // a row that never set it is swallowed: the list box emits no `row-activated`, the row emits
        // no `activated`, and the handler above never runs. Measured on the running window — the row
        // reported `activatable=false` and a synthetic click at its centre produced nothing at all.
        row.set_activatable(true);
        row.set_tooltip_text(Some(&device.name));
        row.add_prefix(&widgets::device_glyph(device.kind));
        // One line for the name, one for the reading, so every row is exactly as tall as every other:
        // the battery bars line up down the column and the list stops re-flowing when a device joins
        // or leaves. A long vendor name is cut with an ellipsis and the whole of it stays available —
        // in this tooltip, and as the heading of the pane beside the rail.
        row.set_title_lines(1);
        row.set_subtitle_lines(1);

        let bar = gtk4::LevelBar::new();
        bar.set_mode(gtk4::LevelBarMode::Continuous);
        // 56, not 72: the bar only has to read as a proportion, and every pixel it gives back is a
        // pixel the one-line name keeps before its ellipsis.
        bar.set_size_request(56, -1);
        bar.set_valign(Align::Center);
        bar.set_value(f64::from(device.percent.unwrap_or(0)) / 100.0);
        bar.set_visible(device.percent.is_some());
        let percent = gtk4::Label::builder()
            .label(&device.percent_text)
            .valign(Align::Center)
            .build();
        percent.add_css_class("numeric");
        let trailing = gtk4::Box::new(Orientation::Horizontal, 8);
        trailing.set_valign(Align::Center);
        trailing.append(&bar);
        trailing.append(&percent);
        row.add_suffix(&trailing);

        self.list.append(&row);
        self.rows.borrow_mut().push(Row {
            key: device.key.clone(),
            row,
            bar,
            percent,
        });
    }
}

/// `Discharging · Mouse · 10s ago` — the device's own status, its kind, and how old the reading is.
fn summary(device: &DeviceView, now: SystemTime) -> String {
    let age = match device.captured {
        Some(captured) => human_age(captured, now),
        None => "never".to_string(),
    };
    format!("{} · {} · {}", device.status, device.kind_label, age)
}

/// `just now · 12:23 17/10/2026` — when the readings in this list were taken.
///
/// This is the "last updated" line: in words, then on the clock, so the row does not count seconds
/// and the exact time is still there to read. It is the one place in the rail with room for both —
/// a row's own line is clamped to one line (`…`), so a row carries only the coarse age.
fn taken_label(snapshot: &Snapshot, now: SystemTime) -> String {
    match snapshot.taken {
        Some(taken) => crate::widgets::updated_text(taken, now),
        None => "never".to_string(),
    }
}

/// Count the descendants of `node` whose GObject type is `name`.
fn count_type(node: &gtk4::Widget, name: &str) -> usize {
    let mut total = usize::from(node.type_().name() == name);
    let mut child = node.first_child();
    while let Some(widget) = child {
        total += count_type(&widget, name);
        child = widget.next_sibling();
    }
    total
}
