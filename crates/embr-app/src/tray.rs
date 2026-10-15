//! The StatusNotifierItem: tooltip, left-click and the device submenu.
//!
//! ## The Noctalia caveat
//!
//! This is a plain, correct SNI item: it takes its own well-known bus name
//! (`org.kde.StatusNotifierItem-<pid>-<n>`) and registers with
//! `org.kde.StatusNotifierWatcher`. On this desktop (KineticWE / the Noctalia shell) that is
//! *registered but not drawn*: `[widget.tray] pinned` in
//! `~/.config/noctalia-kwe/noctalia-config.toml` lists the application ids the bar draws, and
//! anything else lands in the hover-only drawer. Nothing here works around that, because the
//! workaround would be to impersonate another application's id. See the crate README for the
//! one line a user adds.

use std::sync::mpsc::Sender;

use embr_core::Snapshot;
use ksni::menu::{MenuItem, StandardItem, SubMenu};
use ksni::{Icon, ToolTip, Tray};

use crate::messages::{Command, UiRequest};

/// Toolbar pictogram: a battery body with the usual nub, coloured by the worst device.
const ICON_SIDE: i32 = 22;

/// The 3×5 bitmap of each digit, top row first, leftmost column the high bit. Hand-drawn for
/// the same reason the battery is: no icon theme, no asset, and the shape scales with the icon.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b010, 0b010, 0b010], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
];
const DIGIT_WIDTH: i32 = 3;
const DIGIT_HEIGHT: i32 = 5;

pub struct EmbrTray {
    /// `Snapshot::tray_tooltip()` — every device with its level, one per line.
    pub tooltip: String,
    /// One row per device, exactly as the tooltip writes them.
    pub devices: Vec<String>,
    /// The worst level on the machine, for the icon colour.
    pub level: Option<u8>,
    /// Draw that level as a number in the icon instead of as a battery.
    pub shows_percent: bool,
    commands: Sender<Command>,
    ui: Sender<UiRequest>,
}

impl EmbrTray {
    pub fn new(commands: Sender<Command>, ui: Sender<UiRequest>) -> Self {
        Self {
            tooltip: "No Logitech devices found".to_string(),
            devices: Vec::new(),
            level: None,
            shows_percent: false,
            commands,
            ui,
        }
    }

    /// Take the latest snapshot into the item. Called from the device thread only.
    pub fn observe(&mut self, snapshot: &Snapshot, shows_percent: bool) {
        self.tooltip = snapshot.tray_tooltip();
        self.devices = labels(snapshot);
        self.level = snapshot.lowest().and_then(|device| device.percent);
        self.shows_percent = shows_percent;
    }

    /// The pictogram the item should be drawn as, in this build's own style.
    fn pictogram(&self) -> Icon {
        match (self.shows_percent, self.level) {
            // A number is only worth drawing when there is one; with nothing to read the
            // battery glyph says "no reading" better than an empty box would.
            (true, Some(percent)) => percent_icon(percent),
            _ => battery_icon(self.level),
        }
    }

    fn show_window(&self) {
        if self.ui.send(UiRequest::ShowWindow).is_err() {
            eprintln!("[tray] the window is gone; nothing to show");
        }
    }

    fn refresh(&self) {
        // A menu click is the user asking, exactly like the Rescan button: the sweep that
        // follows gives a device that has not answered yet the longer waits.
        if self.commands.send(Command::Rescan).is_err() {
            eprintln!("[tray] the device thread has stopped; cannot rescan");
        }
    }
}

/// `NAME — 88% (Discharging)`, the same shape `core` uses for the tooltip.
pub fn labels(snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .devices
        .iter()
        .map(|device| {
            format!(
                "{} — {} ({})",
                device.name, device.percent_text, device.status
            )
        })
        .collect()
}

impl Tray for EmbrTray {
    /// The application id. Noctalia draws an item only when this exact string is pinned.
    fn id(&self) -> String {
        "embr".to_string()
    }

    fn title(&self) -> String {
        "Embr".to_string()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }

    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }

    /// Left empty on purpose: the pixmap below is what the bar should draw, so no icon theme
    /// is required.
    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![self.pictogram()]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: String::new(),
            icon_pixmap: vec![self.pictogram()],
            title: "Embr".to_string(),
            description: self.tooltip.clone(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.show_window();
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.refresh();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: "Show devices".to_string(),
                activate: Box::new(|tray: &mut Self| tray.show_window()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
        ];

        // One entry per device. Selecting one brings up the window: the level is already in
        // the label, and the window is where anything can actually be done about it.
        let entries: Vec<MenuItem<Self>> = if self.devices.is_empty() {
            vec![
                StandardItem {
                    label: "No Logitech devices found".to_string(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            ]
        } else {
            self.devices
                .iter()
                .map(|label| {
                    StandardItem {
                        label: label.clone(),
                        activate: Box::new(|tray: &mut Self| tray.show_window()),
                        ..Default::default()
                    }
                    .into()
                })
                .collect()
        };

        items.push(
            SubMenu {
                label: "Devices".to_string(),
                submenu: entries,
                ..Default::default()
            }
            .into(),
        );

        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Rescan".to_string(),
                activate: Box::new(|tray: &mut Self| tray.refresh()),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Quit".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    if tray.ui.send(UiRequest::Quit).is_err() {
                        eprintln!("[tray] the window is gone; exiting anyway");
                        std::process::exit(0);
                    }
                }),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}

/// The colour a level is drawn in: red, amber, green — grey when there is nothing to read.
/// One place, so the battery glyph and the percentage badge can never disagree.
fn level_color(level: Option<u8>) -> (u8, u8, u8) {
    match level {
        None => (140u8, 145u8, 150u8),
        Some(percent) if percent <= 20 => (242, 114, 111),
        Some(percent) if percent <= 30 => (244, 193, 80),
        Some(_) => (61, 220, 151),
    }
}

/// A 22×22 ARGB32 battery pictogram (green, amber, red or hollow), drawn by hand so the item
/// needs no icon theme and no image file.
fn battery_icon(level: Option<u8>) -> Icon {
    let (r, g, b) = level_color(level);

    const W: i32 = ICON_SIDE;
    const H: i32 = ICON_SIDE;
    let (body_x0, body_x1) = (2, 17);
    let (body_y0, body_y1) = (6, 16);
    let (nub_x0, nub_x1) = (17, 20);
    let (nub_y0, nub_y1) = (9, 13);

    let mut data: Vec<u8> = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let in_body = x >= body_x0 && x < body_x1 && y >= body_y0 && y < body_y1;
            let in_nub = x >= nub_x0 && x < nub_x1 && y >= nub_y0 && y < nub_y1;
            let on_border =
                in_body && (x == body_x0 || x == body_x1 - 1 || y == body_y0 || y == body_y1 - 1);
            // ARGB32 in network byte order.
            let pixel = if in_nub {
                [255, r, g, b]
            } else if on_border {
                [255, 220, 228, 232]
            } else if in_body {
                [255, r / 2, g / 2, b / 2]
            } else {
                [0, 0, 0, 0]
            };
            data.extend_from_slice(&pixel);
        }
    }

    Icon {
        width: W,
        height: H,
        data,
    }
}

/// The same 22×22 box, drawn as the percentage itself — the number a user actually wants at a
/// glance, in the colour the battery glyph would have been.
///
/// The digits come from the 3×5 table above, at the largest whole-number scale that keeps the
/// whole number inside the icon: one digit is drawn big, and 100 % (three digits) smaller
/// rather than being pushed out of the box.
fn percent_icon(percent: u8) -> Icon {
    const W: i32 = ICON_SIDE;
    const H: i32 = ICON_SIDE;

    let percent = percent.min(100);
    let (r, g, b) = level_color(Some(percent));
    let digits = decimal_digits(percent);
    let scale = scale_for(digits.len() as i32);
    let gap = scale;
    let width = digits.len() as i32 * DIGIT_WIDTH * scale + (digits.len() as i32 - 1) * gap;
    let height = DIGIT_HEIGHT * scale;
    let left = (W - width) / 2;
    let top = (H - height) / 2;

    let mut data: Vec<u8> = vec![0; (W * H * 4) as usize];
    for (index, digit) in digits.iter().enumerate() {
        let bitmap = DIGITS[*digit as usize];
        let origin = left + index as i32 * (DIGIT_WIDTH * scale + gap);
        for (row, bits) in bitmap.iter().enumerate() {
            for column in 0..DIGIT_WIDTH {
                if bits & (0b100 >> column) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let x = origin + column * scale + dx;
                        let y = top + row as i32 * scale + dy;
                        if x < 0 || y < 0 || x >= W || y >= H {
                            continue;
                        }
                        let offset = ((y * W + x) * 4) as usize;
                        data[offset] = 255;
                        data[offset + 1] = r;
                        data[offset + 2] = g;
                        data[offset + 3] = b;
                    }
                }
            }
        }
    }

    Icon {
        width: W,
        height: H,
        data,
    }
}

fn decimal_digits(percent: u8) -> Vec<u8> {
    if percent == 0 {
        return vec![0];
    }
    let mut digits = Vec::new();
    let mut left = percent;
    while left > 0 {
        digits.push(left % 10);
        left /= 10;
    }
    digits.reverse();
    digits
}

/// The largest whole-number scale at which `count` digits and their gaps still fit the icon.
fn scale_for(count: i32) -> i32 {
    let mut best = 1;
    for candidate in 1..=DIGIT_HEIGHT {
        let width = count * DIGIT_WIDTH * candidate + (count - 1) * candidate;
        if width <= ICON_SIDE && DIGIT_HEIGHT * candidate <= ICON_SIDE {
            best = candidate;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use embr_core::DeviceView;

    fn view(percent: Option<u8>) -> DeviceView {
        DeviceView {
            key: "g703".to_string(),
            name: "G703 LIGHTSPEED".to_string(),
            kind: "Mouse".to_string(),
            connection: "USB".to_string(),
            percent,
            percent_text: percent
                .map(|value| format!("{value}%"))
                .unwrap_or_else(|| "—".to_string()),
            estimated: false,
            millivolts: Some(3800),
            status: "Discharging".to_string(),
            charging: false,
            faulty: false,
            alert_below: None,
            alert_below_effective: 20,
            error: None,
            captured: None,
        }
    }

    fn snapshot(devices: Vec<DeviceView>) -> Snapshot {
        Snapshot {
            devices,
            taken: None,
            warnings: Vec::new(),
        }
    }

    /// Opaque pixels in a pixmap, as a set of coordinates — what a test can compare without
    /// caring about the order the pixels were written in.
    fn opaque(icon: &Icon) -> Vec<(i32, i32)> {
        let mut points = Vec::new();
        for y in 0..icon.height {
            for x in 0..icon.width {
                let offset = ((y * icon.width + x) * 4) as usize;
                if icon.data[offset] != 0 {
                    points.push((x, y));
                }
            }
        }
        points
    }

    fn tray(level: Option<u8>, shows_percent: bool) -> EmbrTray {
        let (commands, _) = std::sync::mpsc::channel();
        let (ui, _) = std::sync::mpsc::channel();
        let mut tray = EmbrTray::new(commands, ui);
        tray.observe(&snapshot(vec![view(level)]), shows_percent);
        tray
    }

    #[test]
    fn every_percentage_is_drawn_and_stays_inside_the_icon() {
        for percent in 0..=100u8 {
            let icon = percent_icon(percent);
            assert_eq!(icon.width, ICON_SIDE);
            assert_eq!(icon.height, ICON_SIDE);
            assert_eq!(icon.data.len(), (ICON_SIDE * ICON_SIDE * 4) as usize);
            let points = opaque(&icon);
            assert!(!points.is_empty(), "{percent} % drew nothing");
            assert!(
                points
                    .iter()
                    .all(|(x, y)| (0..ICON_SIDE).contains(x) && (0..ICON_SIDE).contains(y)),
                "{percent} % drew outside the icon"
            );
        }
    }

    #[test]
    fn the_digits_are_scaled_to_fit_and_drawn_once() {
        assert_eq!(scale_for(1), 4, "one digit is drawn big");
        assert_eq!(scale_for(2), 3);
        assert_eq!(scale_for(3), 2, "100 % still has to fit");
        for percent in [7u8, 57, 100] {
            let digits = decimal_digits(percent);
            let scale = scale_for(digits.len() as i32);
            let expected: usize = digits
                .iter()
                .map(|digit| {
                    DIGITS[*digit as usize]
                        .iter()
                        .map(|bits| bits.count_ones() as usize)
                        .sum::<usize>()
                        * (scale * scale) as usize
                })
                .sum();
            assert_eq!(
                opaque(&percent_icon(percent)).len(),
                expected,
                "{percent} % must draw each set bit exactly once"
            );
        }
    }

    #[test]
    fn a_number_is_only_drawn_when_it_was_asked_for() {
        let battery = tray(Some(7), false).pictogram();
        assert_eq!(battery.data, battery_icon(Some(7)).data);

        let number = tray(Some(7), true).pictogram();
        assert_eq!(number.data, percent_icon(7).data);
        assert_ne!(number.data, battery.data);
    }

    #[test]
    fn without_a_reading_the_item_keeps_the_battery_glyph() {
        // Nothing to put in a number, and an empty box would read as "no device" rather than
        // "no reading", which is what the grey battery says.
        let icon = tray(None, true).pictogram();
        assert_eq!(icon.data, battery_icon(None).data);
    }

    #[test]
    fn the_icon_takes_the_worst_level_on_the_machine() {
        let (commands, _) = std::sync::mpsc::channel();
        let (ui, _) = std::sync::mpsc::channel();
        let mut tray = EmbrTray::new(commands, ui);
        tray.observe(
            &snapshot(vec![view(Some(88)), view(Some(19)), view(None)]),
            true,
        );
        assert_eq!(tray.level, Some(19));
        assert_eq!(tray.pictogram().data, percent_icon(19).data);
    }
}
