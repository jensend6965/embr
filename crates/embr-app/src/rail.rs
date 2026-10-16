//! The rail: the devices, and under the open one, the pages *it* has.
//!
//! Navigation gains a level here — device, then feature — and the level is the device's own:
//! every row under a device is a page that device advertised (`core::pages_for`), so the rail
//! cannot offer the keyboard a DPI page it has no feature for.
//!
//! Rows are `AdwActionRow`s in a `GtkListBox`. That is not a style choice: an `AdwActionRow`
//! connects itself to its parent list box's `row-activated` signal when it is rooted, which is
//! what makes a click on it emit `activated`. A row that is not a child of a list box looks
//! identical and never fires.

use std::cell::{Cell, RefCell};
use std::sync::mpsc::Sender;
use std::time::SystemTime;

use adw::prelude::*;
use gtk4::{Align, Orientation};

use embr_core::{DeviceView, FeaturePage, Snapshot, human_age};

use crate::messages::UiEvent;
use crate::widgets;

/// How wide the rail is: wide enough for a device name and its percentage, narrow enough that the
/// page keeps the room. The redesign sheet draws the same 340 px at 1240 px.
const RAIL_WIDTH: i32 = 340;

/// One device's card: the device row and, revealed under it, the pages it advertised.
struct Card {
    key: String,
    /// The list box the card's rows live in — the thing a row needs a parent of to be clickable,
    /// and the thing that draws the selection around the open device.
    list: gtk4::ListBox,
    /// The row for the device itself.
    row: adw::ActionRow,
    chevron: gtk4::Image,
    bar: gtk4::LevelBar,
    percent: gtk4::Label,
    /// One row per page, in the order the device listed them.
    pages: RefCell<Vec<(FeaturePage, adw::ActionRow)>>,
    /// Rows that are not pages — "features not read", "some features did not answer". They are
    /// kept so that a rebuild takes them away again, which is the bug this field exists for: a
    /// card that reported "not read" and then read the device would have kept the note.
    notes: RefCell<Vec<adw::ActionRow>>,
    /// The pages this card was last built for, and whether it has been built at all. Both are
    /// needed: with the feature set unknown, `built` is `None` from the first moment, so a card
    /// that compared them alone would never build anything — measured, as an open keyboard with
    /// nothing under it.
    built: RefCell<Option<Vec<FeaturePage>>>,
    drawn: Cell<bool>,
    /// Where a click on one of this card's rows is reported.
    events: Sender<UiEvent>,
}

impl Card {
    /// Whether the rows this card is showing are the ones for the pages it was asked about.
    fn is_built_for(&self, pages: &Option<Vec<FeaturePage>>) -> bool {
        self.drawn.get() && self.built.borrow().as_ref() == pages.as_ref()
    }

    /// Take every row below the device away, pages and notes alike.
    fn clear(&self) {
        for (_, row) in self.pages.borrow_mut().drain(..) {
            self.list.remove(&row);
        }
        for row in self.notes.borrow_mut().drain(..) {
            self.list.remove(&row);
        }
    }

    fn note(&self, row: adw::ActionRow) {
        self.list.append(&row);
        self.notes.borrow_mut().push(row);
    }

    /// Reveal exactly the pages this device has, and say what is not known about them.
    fn show_pages(&self, device: &DeviceView, open: bool) {
        if !self.is_built_for(&device.pages) {
            self.clear();
            match &device.pages {
                Some(pages) => {
                    for page in pages {
                        let row = self.page_row(*page);
                        self.list.append(&row);
                        self.pages.borrow_mut().push((*page, row));
                    }
                    // A list that came back with slots the device never answered is a subset, and
                    // saying so is the difference between "this keyboard has no lighting" and
                    // "this keyboard did not say": the same row, and one of them is a lie.
                    if device.pages_partial {
                        self.note(partial_row(device));
                    }
                }
                // A device whose feature set could not be read has no pages to offer, and an open
                // device with nothing under it reads as a broken window.
                None => self.note(unread_row(device)),
            }
            self.built.replace(device.pages.clone());
            self.drawn.set(true);
        }

        for (page, row) in self.pages.borrow().iter() {
            row.set_visible(open);
            if *page == FeaturePage::Battery {
                row.set_subtitle(&alert_line(device));
            }
        }
        // A note is about the open device, so it goes when the card closes.
        for row in self.notes.borrow().iter() {
            row.set_visible(open);
        }
    }

    fn page_row(&self, page: FeaturePage) -> adw::ActionRow {
        let row = crate::widgets::row(page.label());
        // The indentation is the level: a page belongs to the device above it.
        row.set_margin_start(18);
        let events = self.events.clone();
        let key = self.key.clone();
        row.connect_activated(move |_| {
            let _ = events.send(UiEvent::ShowPage {
                key: key.clone(),
                page,
            });
        });
        row
    }
}

/// The rail, and everything it needs to draw itself again.
pub struct Rail {
    /// The container to put in the window.
    pub container: gtk4::Box,
    cards_box: gtk4::Box,
    scrub: gtk4::Label,
    empty: gtk4::Label,
    cards: RefCell<Vec<Card>>,
    events: Sender<UiEvent>,
}

impl Rail {
    pub fn new(events: Sender<UiEvent>) -> Self {
        let container = gtk4::Box::new(Orientation::Vertical, 0);
        container.set_size_request(RAIL_WIDTH, -1);

        let header = gtk4::Box::new(Orientation::Horizontal, 8);
        header.set_margin_top(14);
        header.set_margin_bottom(6);
        header.set_margin_start(18);
        header.set_margin_end(18);
        let title = gtk4::Label::builder().label("DEVICES").xalign(0.0).build();
        title.add_css_class("caption-heading");
        let scrub = gtk4::Label::builder()
            .label("")
            .xalign(1.0)
            .hexpand(true)
            .build();
        scrub.add_css_class("caption");
        scrub.add_css_class("dim-label");
        header.append(&title);
        header.append(&scrub);

        let cards_box = gtk4::Box::new(Orientation::Vertical, 12);
        cards_box.set_margin_top(6);
        cards_box.set_margin_bottom(18);
        cards_box.set_margin_start(12);
        cards_box.set_margin_end(12);

        // Shown instead of the cards when nothing was found. A sentence rather than an empty box,
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
            .child(&cards_box)
            .vexpand(true)
            .build();

        container.append(&header);
        container.append(&scrolled);
        container.append(&empty);

        Self {
            container,
            cards_box,
            scrub,
            empty,
            cards: RefCell::new(Vec::new()),
            events,
        }
    }

    /// Draw the devices: the rows in snapshot order, the open one expanded, and under it the
    /// pages that device advertised.
    pub fn sync(&self, snapshot: &Snapshot, selected: Option<&str>, now: SystemTime) {
        let wanted: Vec<String> = snapshot
            .devices
            .iter()
            .map(|device| device.key.clone())
            .collect();
        let current: Vec<String> = self
            .cards
            .borrow()
            .iter()
            .map(|card| card.key.clone())
            .collect();
        // Rebuild only when the set or the order changed. A watch pass publishes every few
        // seconds and the order only moves when a battery crosses another, so this is what keeps
        // a rebuild from happening under the user's pointer — the values are written in place
        // otherwise, and only in `refresh_ages` between sweeps.
        if wanted != current {
            self.rebuild();
            for device in &snapshot.devices {
                self.add(device, selected);
            }
        }

        for (card, device) in self.cards.borrow().iter().zip(snapshot.devices.iter()) {
            card.row.set_title(&device.name);
            card.row.set_subtitle(&summary(device, now));
            card.row.set_tooltip_text(Some(&device.name));
            card.percent.set_label(&device.percent_text);
            // The bar carries the level's colour; the percentage beside it stays ordinary text.
            // Repainted on every sync, so flipping the switch shows immediately.
            widgets::paint_level(&card.bar, device);
            card.bar
                .set_value(f64::from(device.percent.unwrap_or(0)) / 100.0);
            card.bar.set_visible(device.percent.is_some());

            let open = selected == Some(device.key.as_str());
            card.list
                .select_row(if open { Some(&card.row) } else { None });
            card.chevron.set_icon_name(Some(if open {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            }));
            card.show_pages(device, open);
        }

        self.scrub.set_label(&taken_label(snapshot, now));
        self.empty.set_visible(snapshot.devices.is_empty());
    }

    /// Keep the ages honest between sweeps: the row subtitles and the scrub time are the only
    /// things here that change while nothing is being read.
    pub fn refresh_ages(&self, snapshot: &Snapshot, now: SystemTime) {
        self.scrub.set_label(&taken_label(snapshot, now));
        for (card, device) in self.cards.borrow().iter().zip(snapshot.devices.iter()) {
            card.row.set_subtitle(&summary(device, now));
        }
    }

    /// Whether the rail is still showing this device — a device that left while it was open.
    pub fn has(&self, key: &str) -> bool {
        self.cards.borrow().iter().any(|card| card.key == *key)
    }

    fn rebuild(&self) {
        self.cards.borrow_mut().clear();
        while let Some(child) = self.cards_box.first_child() {
            self.cards_box.remove(&child);
        }
    }

    fn add(&self, device: &DeviceView, selected: Option<&str>) {
        // A card is a list box of its own: an `AdwActionRow` only reports a click when its parent
        // is a list box, and one box per device is also the shape the design sheet draws — a
        // device and the pages under it on one surface.
        let list = gtk4::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk4::SelectionMode::Single);

        let row = crate::widgets::row_with(&device.name, &summary(device, SystemTime::now()));
        // The click: the list box activates the row, and the row says it was activated.
        let events = self.events.clone();
        let key = device.key.clone();
        row.connect_activated(move |_| {
            let _ = events.send(UiEvent::Select(key.clone()));
        });
        let chevron = gtk4::Image::from_icon_name("pan-end-symbolic");
        chevron.set_valign(Align::Center);
        row.add_prefix(&chevron);

        let bar = gtk4::LevelBar::new();
        bar.set_mode(gtk4::LevelBarMode::Continuous);
        bar.set_size_request(72, -1);
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

        list.append(&row);
        let card = Card {
            key: device.key.clone(),
            list,
            row,
            chevron,
            bar,
            percent,
            pages: RefCell::new(Vec::new()),
            notes: RefCell::new(Vec::new()),
            built: RefCell::new(None),
            drawn: Cell::new(false),
            events: self.events.clone(),
        };
        card.show_pages(device, selected == Some(device.key.as_str()));
        self.cards_box.append(&card.list);
        self.cards.borrow_mut().push(card);
    }
}

/// The row an open device gets when part of its feature set did not answer: the pages above it are
/// the ones it confirmed, not all of them.
fn partial_row(device: &DeviceView) -> adw::ActionRow {
    let row = crate::widgets::row_with(
        "Some features did not answer",
        &format!(
            "{} only answered part of its own feature list — press Rescan to ask it again",
            device.name
        ),
    );
    row.set_activatable(false);
    row
}

/// The row an open device gets when its feature set has not been read: it says which of the two
/// reasons it is, because "nothing here" and "this device is not answering" look identical
/// otherwise. Not a page — nothing to click, which is why it is not activatable.
fn unread_row(device: &DeviceView) -> adw::ActionRow {
    let row = if device.error.is_some() {
        crate::widgets::row_with(
            "Features not read",
            "this device did not answer — press Rescan to ask it again",
        )
    } else {
        crate::widgets::row_with(
            "Features not read",
            "asking this device what it can be asked about",
        )
    };
    row.set_activatable(false);
    row
}

/// `Discharging · Mouse · 10s ago` — the device's own status, its kind, and how old the reading is.
fn summary(device: &DeviceView, now: SystemTime) -> String {
    let age = match device.captured {
        Some(captured) => human_age(captured, now),
        None => "never".to_string(),
    };
    format!("{} · {} · {}", device.status, device.kind, age)
}

/// `its own alert 20%` / `alert 20%` — the level this device's low-battery alert uses: its own,
/// or the shared one it falls back on.
fn alert_line(device: &DeviceView) -> String {
    format!(
        "{}alert {}%",
        if device.alert_below.is_some() {
            "its own "
        } else {
            ""
        },
        device.alert_below_effective
    )
}

fn taken_label(snapshot: &Snapshot, now: SystemTime) -> String {
    match snapshot.taken {
        Some(taken) => human_age(taken, now),
        None => "never".to_string(),
    }
}
