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

pub struct EmbrTray {
    /// `Snapshot::tray_tooltip()` — every device with its level, one per line.
    pub tooltip: String,
    /// One row per device, exactly as the tooltip writes them.
    pub devices: Vec<String>,
    /// The worst level on the machine, for the icon colour.
    pub level: Option<u8>,
    commands: Sender<Command>,
    ui: Sender<UiRequest>,
}

impl EmbrTray {
    pub fn new(commands: Sender<Command>, ui: Sender<UiRequest>) -> Self {
        Self {
            tooltip: "No Logitech devices found".to_string(),
            devices: Vec::new(),
            level: None,
            commands,
            ui,
        }
    }

    /// Take the latest snapshot into the item. Called from the device thread only.
    pub fn observe(&mut self, snapshot: &Snapshot) {
        self.tooltip = snapshot.tray_tooltip();
        self.devices = labels(snapshot);
        self.level = snapshot.lowest().and_then(|device| device.percent);
    }

    fn show_window(&self) {
        if self.ui.send(UiRequest::ShowWindow).is_err() {
            eprintln!("[tray] the window is gone; nothing to show");
        }
    }

    fn refresh(&self) {
        if self.commands.send(Command::Refresh).is_err() {
            eprintln!("[tray] the device thread has stopped; cannot refresh");
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
        vec![battery_icon(self.level)]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: String::new(),
            icon_pixmap: vec![battery_icon(self.level)],
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
                label: "Refresh".to_string(),
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

/// A 22×22 ARGB32 battery pictogram (green, amber, red or hollow), drawn by hand so the item
/// needs no icon theme and no image file.
fn battery_icon(level: Option<u8>) -> Icon {
    let (r, g, b) = match level {
        None => (140u8, 145u8, 150u8),
        Some(percent) if percent <= 20 => (242, 114, 111),
        Some(percent) if percent <= 30 => (244, 193, 80),
        Some(_) => (61, 220, 151),
    };

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
