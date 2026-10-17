//! The StatusNotifierItems: one per icon, with their tooltips, click handling and device submenu.
//!
//! ## One icon, or several
//!
//! The user may ask for an icon per device (the settings page lists them), in which case each
//! registered item is about one device and shows *its* level — the shape a monitoring overlay has
//! when it puts a readout per sensor on screen. With nothing chosen there is one icon, and it
//! follows the device with the least charge. Which icons should exist is decided in
//! [`embr_core::tray_icons`]; this module only draws them and keeps the live set in step.
//!
//! ## The drawing
//!
//! Every pictogram is 22×22 ARGB32, drawn in code by hand — no icon theme, no image file. The
//! shapes that are not text come from one template ([`FLAT_BATTERY`] / [`UPRIGHT_BATTERY`], painted
//! by [`battery_body`]): a rounded body and a nub, drawn as a white line two units thick, with the
//! level as how much of the tube is filled. Coordinates are whole units on purpose — at this size a
//! unit is a pixel, and a line on half-pixel centres anti-aliases into three columns of half
//! coverage and reads as grey rather than as white. Colour appears in exactly one place, the digits
//! of the `percent` style, and which colour that is comes from [`TrayTextColor`].
//!
//! ## The Noctalia caveat
//!
//! These are plain, correct SNI items: each takes its own well-known bus name
//! (`org.kde.StatusNotifierItem-<pid>-<n>`) and registers with `org.kde.StatusNotifierWatcher`. On
//! this desktop (KineticWE / the Noctalia shell) that is *registered but not drawn*: `[widget.tray]`
//! `pinned` in `~/.config/noctalia-kwe/noctalia-config.toml` lists the application ids the bar
//! draws, and anything else lands in the hover-only drawer. An icon per device therefore needs each
//! id pinned — `embr` for the one that follows the worst device, `embr-<device key>` for the rest.
//! Nothing here works around that, because the workaround would be to impersonate another
//! application's id. See the crate README for the lines a user adds.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use embr_core::{DIM, Snapshot, TrayIcon, TrayLook, TrayStyle, TrayTextColor, WHITE};
use ksni::menu::{MenuItem, StandardItem, SubMenu};
use ksni::{Handle, Icon, ToolTip, Tray};

use crate::TrayAction;

/// Toolbar pictogram: one template for every shape, in the icon's own 22-unit square.
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

/// One tray icon.
pub struct EmbrTray {
    /// The StatusNotifierItem id, from [`TrayIcon::id`]: the bar draws an item only if this exact
    /// string is pinned.
    id: String,
    /// The device this icon is about, if it is about one.
    key: Option<String>,
    /// The tooltip's title: the app's name, or the device's when the icon is that device's.
    title: String,
    /// The tooltip's body.
    tooltip: String,
    /// The level to draw: this icon's own device, or the worst one.
    level: Option<u8>,
    /// Whether the device the level came from is on external power.
    charging: bool,
    /// Whether that device reports a faulty battery.
    faulty: bool,
    /// Which shape to draw it as, and the colour of the text when the style has any.
    look: TrayLook,
    /// Every device, for the menu.
    devices: Vec<String>,
    /// Where a click on this icon is reported.
    actions: Sender<TrayAction>,
}

impl EmbrTray {
    pub fn new(id: String, actions: Sender<TrayAction>) -> Self {
        Self {
            id,
            key: None,
            title: "Embr".to_string(),
            tooltip: "No Logitech devices found".to_string(),
            level: None,
            charging: false,
            faulty: false,
            look: TrayLook::default(),
            devices: Vec::new(),
            actions,
        }
    }

    /// Take the latest reading for this icon. Called from the device thread only.
    pub fn observe(&mut self, icon: &TrayIcon, devices: &[String], look: TrayLook) {
        self.key = icon.key.clone();
        self.title = icon.title.clone();
        self.tooltip = icon.detail.clone();
        self.level = icon.level;
        self.charging = icon.charging;
        self.faulty = icon.faulty;
        self.look = look;
        self.devices = devices.to_vec();
    }

    /// What the icon is drawn as, which is presentation and not a reading: the window sets this the
    /// moment a picker moves rather than waiting for the device thread, which may be seconds into a
    /// sweep. See [`Icons::restyle`].
    pub fn set_look(&mut self, look: TrayLook) {
        self.look = look;
    }

    /// The pictogram the item should be drawn as, in this build's own style.
    fn pictogram(&self) -> Icon {
        glyph(self.look, self.level, self.charging, self.faulty)
    }

    fn show_window(&self) {
        // An icon that is about one device is a click on that device: the window opens on the
        // hardware the icon was reporting, not on whatever was selected last.
        let request = TrayAction::ShowWindow {
            key: self.key.clone(),
        };
        if self.actions.send(request).is_err() {
            eprintln!("[tray] the window is gone; nothing to show");
        }
    }

    fn refresh(&self) {
        // A menu click is the user asking, exactly like the Rescan button: the sweep that
        // follows gives a device that has not answered yet the longer waits.
        if self.actions.send(TrayAction::Rescan).is_err() {
            eprintln!("[tray] the window is gone; cannot rescan");
        }
    }
}

/// One live icon: the id it registered as, and the handle that updates it.
type Live = (String, Handle<EmbrTray>);

/// The tray icons that are live right now.
///
/// Shared between the two threads that have something to say about them: the device thread feeds
/// them readings and keeps the set in step with the configuration, and the window flips their
/// *style* the moment the switch moves. That split is deliberate — a style change has nothing to
/// do with the devices, and routing it through the device thread meant waiting out whatever sweep
/// was in flight (measured: 827 ms when one was running, 18 ms when it was not).
#[derive(Clone)]
pub struct Icons {
    live: Arc<Mutex<Vec<Live>>>,
    actions: Sender<TrayAction>,
}

impl Icons {
    /// The senders are kept here rather than passed on every call: this is the one place that makes
    /// tray items, and an item needs a way to ask for a rescan and a way to show the window.
    pub fn new(actions: Sender<TrayAction>) -> Self {
        Self {
            live: Arc::new(Mutex::new(Vec::new())),
            actions,
        }
    }

    /// Make the live icons match what the configuration asks for.
    ///
    /// Called from the device thread, which is what holds readings. An icon that is no longer
    /// asked for is shut down — its name comes off the bus and the bar forgets it — and an icon
    /// that is new is registered there and then, so a choice on the settings page takes effect
    /// without a restart.
    pub fn sync(&self, wanted: &[TrayIcon], menu: &[String], look: TrayLook) {
        let mut live = self
            .live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let wanted_ids: Vec<String> = wanted.iter().map(TrayIcon::id).collect();
        live.retain(|(id, handle)| {
            let still_wanted = wanted_ids.contains(id);
            if !still_wanted {
                eprintln!("[tray] \"{id}\" is no longer asked for: its icon is gone from the bar");
                handle.shutdown();
            }
            still_wanted
        });

        for icon in wanted {
            let id = icon.id();
            match live.iter().find(|(live_id, _)| *live_id == id) {
                Some((_, handle)) => handle.update(|item| item.observe(icon, menu, look)),
                None => {
                    let mut item = EmbrTray::new(id.clone(), self.actions.clone());
                    item.observe(icon, menu, look);
                    let service = ksni::TrayService::new(item);
                    let handle = service.handle();
                    // ksni runs each item on its own thread, and unwraps its own registration
                    // there: a bar that refuses the item loses that icon, not the application.
                    service.spawn();
                    eprintln!("[tray] registered the icon \"{id}\" (pin it in your bar to see it)");
                    live.push((id.clone(), handle));
                }
            }
        }
    }

    /// What the icons look like, straight from the UI thread: the options the user just picked,
    /// applied to every icon at once. Readings and the set of icons stay the device thread's
    /// business.
    pub fn restyle(&self, look: TrayLook) {
        let live = self
            .live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (_, handle) in live.iter() {
            handle.update(|item| item.set_look(look));
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
        self.id.clone()
    }

    fn title(&self) -> String {
        self.title.clone()
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
            title: self.title.clone(),
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
                    if tray.actions.send(TrayAction::Quit).is_err() {
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

/// A rectangle in the icon's own units: x0, x1, y0, y1.
///
/// Whole numbers, and that is deliberate: the box is 22 units across so a unit is a pixel, and an
/// edge that lands on a pixel boundary comes out crisp. A thinner line drawn on half-pixels is the
/// one thing this template cannot afford — it is antialiased over three columns of half coverage
/// and reads as grey rather than as white.
type Rect = (f32, f32, f32, f32);

/// The template every shape is drawn from: a body with a rounded outline, and the nub that makes it
/// a battery, in whichever orientation the style asks for. x0, x1, y0, y1 of each, in the icon's
/// 22-unit square.
///
/// Deliberately *not* a copy of anyone's battery: the proportions are this app's — a body three
/// parts of the width and half of that tall, a nub a fifth of it, a line two units thick, corners
/// rounded to three — and it is drawn as a line rather than as a filled slab. Monochrome, because
/// the shapes carry the level and the only colour a tray icon has belongs to the text of the
/// percentage style.
const FLAT_BATTERY: (Rect, Rect) = ((3.0, 19.0, 6.0, 16.0), (19.0, 21.0, 9.0, 13.0));
/// The same battery standing on its base, terminal up, which is how a battery is stood up.
const UPRIGHT_BATTERY: (Rect, Rect) = ((6.0, 16.0, 3.0, 19.0), (9.0, 13.0, 1.0, 3.0));
/// How round the body's corners are, and how thick the line is.
const CORNER: f32 = 3.0;
const LINE: f32 = 2.0;
/// How far the fill keeps off the line, and how round its own corners are.
const CLEARANCE: f32 = 0.75;

/// The body and nub of one orientation, painted by `inside` where the line is not: the caller fills
/// the level in, or draws a bolt on it, or draws nothing at all. One function, so every style that
/// wears this shape wears the same one.
fn battery_body(upright: bool, inside: impl Fn(f32, f32) -> Option<(u8, u8, u8)>) -> Icon {
    let (body, nub) = if upright {
        UPRIGHT_BATTERY
    } else {
        FLAT_BATTERY
    };
    shape_icon(|x, y| {
        if in_rounded(x, y, nub, CORNER * 0.5) {
            return Some(WHITE);
        }
        if !in_rounded(x, y, body, CORNER) {
            return None;
        }
        if !in_rounded(x, y, inset(body, LINE), CORNER - LINE) {
            // On the line itself.
            return Some(WHITE);
        }
        inside(x, y)
    })
}

/// The level as how much of the tube is filled — or, for a style with nothing to read, as an empty
/// tube, which is this app's word for `no level`.
fn battery_icon(level: Option<u8>, upright: bool) -> Icon {
    let fraction = fraction_of(level);
    battery_body(upright, |x, y| {
        filled(x, y, upright, fraction).then_some(WHITE)
    })
}

/// Whether a point is inside a rounded rectangle. `radius` is the corner radius, and it is clamped
/// so a rect that is thinner than its own corners still has a shape.
fn in_rounded(x: f32, y: f32, rect: Rect, radius: f32) -> bool {
    let (x0, x1, y0, y1) = rect;
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let radius = radius.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0);
    let (cx, cy) = (
        x.clamp(x0 + radius, x1 - radius),
        y.clamp(y0 + radius, y1 - radius),
    );
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= radius * radius
}

/// The same rectangle, moved in by `by` on every side.
fn inset(rect: Rect, by: f32) -> Rect {
    (rect.0 + by, rect.1 - by, rect.2 + by, rect.3 - by)
}

/// Whether a point is inside the level's fill: the tube's full width (or height) and the first
/// `fraction` of its length, measured from the left of a flat battery and from the base of an
/// upright one.
///
/// The fill keeps a hairline off the line all round, so the outline still reads as an outline.
fn filled(x: f32, y: f32, upright: bool, fraction: f32) -> bool {
    if fraction <= 0.0 {
        return false;
    }
    let (body, _) = if upright {
        UPRIGHT_BATTERY
    } else {
        FLAT_BATTERY
    };
    // The fill sits inside the line with a hairline of clearance and its own rounded corners. A fill
    // that touched the line would merge with the left wall of the tube and the outline would stop
    // reading as an outline — the shape would look like a solid block with a notch in it. The
    // clearance is proportional to the tube rather than absolute, so a battery stood on end is not
    // filled any thinner than one lying down.
    let (x0, x1, y0, y1) = inset(inset(body, LINE), CLEARANCE);
    if upright {
        let height = (y1 - y0) * fraction.min(1.0);
        in_rounded(x, y, (x0, x1, y1 - height, y1), CLEARANCE * 1.5)
    } else {
        let width = (x1 - x0) * fraction.min(1.0);
        in_rounded(x, y, (x0, x0 + width, y0, y1), CLEARANCE * 1.5)
    }
}

/// The level as a fraction of full, with nothing to read counting as nothing to draw.
fn fraction_of(level: Option<u8>) -> f32 {
    level
        .map(|percent| f32::from(percent.min(100)) / 100.0)
        .unwrap_or(0.0)
}

/// The pictogram for one style, one reading and one device state — the single place that decides
/// what the bar draws, so the settings page's previews and the live icon cannot disagree.
///
/// The shapes are monochrome line drawings: the level is how much of the box is filled, which
/// needs no colour, and the one style that has colour at all is the percentage, whose digits take
/// the colour the user chose.
pub fn glyph(look: TrayLook, level: Option<u8>, charging: bool, faulty: bool) -> Icon {
    match look.style {
        TrayStyle::Battery => battery_icon(level, false),
        TrayStyle::BatteryVertical => battery_icon(level, true),
        TrayStyle::Percent => match level {
            Some(percent) => percent_icon(percent, look.text_color),
            None => battery_icon(None, false),
        },
        TrayStyle::Ring => match level {
            Some(percent) => ring_icon(percent),
            None => battery_icon(None, false),
        },
        TrayStyle::Status => status_icon(level, charging, faulty),
    }
}

/// The pictogram a style draws for a sample reading — what the settings page shows in the picker.
///
/// It goes through [`glyph`] like the live icon does, so a preview is the drawing itself rather
/// than a second copy of it that can fall behind. The sample is chosen to show what the style is
/// for: the `status` one is previewed *while charging*, because that is the thing it does that the
/// others do not.
pub fn preview(style: TrayStyle, text_color: TrayTextColor) -> Icon {
    let look = TrayLook { style, text_color };
    match style {
        TrayStyle::Status => glyph(look, Some(65), true, false),
        TrayStyle::Percent => glyph(look, Some(65), false, false),
        _ => glyph(look, Some(35), false, false),
    }
}

/// A ring whose arc is the level: the style for a panel where a circle reads better than a bar.
///
/// The arc starts at twelve o'clock and runs clockwise in white, and the rest of the ring is drawn
/// in [`DIM`] rather than left out — a gap would look like a partly drawn icon. Every pixel is the
/// average of 3×3 samples, because a circle at 22 px is otherwise a staircase.
fn ring_icon(percent: u8) -> Icon {
    const CENTER: f32 = (ICON_SIDE as f32 - 1.0) / 2.0;
    const OUTER: f32 = 9.0;
    const INNER: f32 = 6.4;

    let fraction = fraction_of(Some(percent));
    shape_icon(|x, y| {
        let (dx, dy) = (x - CENTER, y - CENTER);
        let radius = (dx * dx + dy * dy).sqrt();
        if radius > OUTER || radius < INNER {
            return None;
        }
        // `atan2(dx, -dy)` is zero at twelve o'clock and grows clockwise.
        let turn = (dx.atan2(-dy) / std::f32::consts::TAU + 1.0) % 1.0;
        Some(if turn <= fraction { WHITE } else { DIM })
    })
}

/// The battery template plus the one thing a level cannot say: a bolt while the device is on
/// external power, an exclamation mark when the device itself reports a broken battery.
///
/// Both are drawn in the same white as the outline — a warning that needed red would be the only
/// colour in the tray, and the percentage style is where colour belongs.
fn status_icon(level: Option<u8>, charging: bool, faulty: bool) -> Icon {
    let fraction = fraction_of(level);
    battery_body(false, |x, y| {
        if faulty {
            // The fault replaces the fill: a level is not worth reading until a battery the device
            // calls broken is dealt with.
            let bar = (10.0..12.0).contains(&x) && (9.0..11.4).contains(&y);
            let dot = (10.0..12.0).contains(&x) && (12.4..14.0).contains(&y);
            return (bar || dot).then_some(WHITE);
        }
        if charging {
            // The bolt replaces the fill rather than sitting on top of it: a white shape on a
            // white fill is no shape at all.
            return inside_polygon(&BOLT, x, y).then_some(WHITE);
        }
        filled(x, y, false, fraction).then_some(WHITE)
    })
}

/// The bolt, sized to the tube it stands in: a six-point lightning shape, drawn as a polygon rather
/// than a bitmap so its diagonals can be anti-aliased like the ring's arc.
const BOLT: [(f32, f32); 6] = [
    (12.6, 8.1),
    (9.0, 11.4),
    (10.8, 11.4),
    (9.6, 13.9),
    (13.2, 10.4),
    (11.3, 10.4),
];

/// A 22×22 pixmap from a shape: `Some(rgb)` where the shape covers the point, averaged over 3×3
/// samples per pixel so a curve or a diagonal is not a staircase. The pixel that comes out carries
/// real alpha, which is what makes these two look drawn rather than bitmapped.
fn shape_icon(shape: impl Fn(f32, f32) -> Option<(u8, u8, u8)>) -> Icon {
    const SAMPLES: i32 = 3;
    const TOTAL: u32 = (SAMPLES * SAMPLES) as u32;
    let mut data = vec![0u8; (ICON_SIDE * ICON_SIDE * 4) as usize];
    for y in 0..ICON_SIDE {
        for x in 0..ICON_SIDE {
            let (mut r, mut g, mut b, mut hits) = (0u32, 0u32, 0u32, 0u32);
            for row in 0..SAMPLES {
                for column in 0..SAMPLES {
                    let px = x as f32 + (column as f32 + 0.5) / SAMPLES as f32;
                    let py = y as f32 + (row as f32 + 0.5) / SAMPLES as f32;
                    if let Some((cr, cg, cb)) = shape(px, py) {
                        r += u32::from(cr);
                        g += u32::from(cg);
                        b += u32::from(cb);
                        hits += 1;
                    }
                }
            }
            if hits == 0 {
                continue;
            }
            let offset = ((y * ICON_SIDE + x) * 4) as usize;
            data[offset] = (255 * hits / TOTAL) as u8;
            data[offset + 1] = (r / hits) as u8;
            data[offset + 2] = (g / hits) as u8;
            data[offset + 3] = (b / hits) as u8;
        }
    }
    Icon {
        width: ICON_SIDE,
        height: ICON_SIDE,
        data,
    }
}

/// Whether a point is inside a polygon, by the even-odd rule — all a bolt needs, and it keeps the
/// shape readable as the list of points above rather than as a bitmap nobody can check.
fn inside_polygon(points: &[(f32, f32)], x: f32, y: f32) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = points.len() - 1;
    for current in 0..points.len() {
        let (xi, yi) = points[current];
        let (xj, yj) = points[previous];
        // `&&` short-circuits, so the division cannot happen when the edge is horizontal.
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

/// The same 22×22 box, drawn as the percentage itself — the number a user actually wants at a
/// glance, in the colour the user asked for (by level, or one fixed colour).
///
/// The digits come from the 3×5 table above, at the largest whole-number scale that keeps the
/// whole number inside the icon: one digit is drawn big, and 100 % (three digits) smaller
/// rather than being pushed out of the box.
fn percent_icon(percent: u8, color: TrayTextColor) -> Icon {
    const W: i32 = ICON_SIDE;
    const H: i32 = ICON_SIDE;

    let percent = percent.min(100);
    let (r, g, b) = color.color(Some(percent));
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
    use embr_core::{DeviceKind, DeviceView};

    fn view(key: &str, percent: Option<u8>) -> DeviceView {
        DeviceView {
            key: key.to_string(),
            name: format!("G703 {key}"),
            kind: DeviceKind::Mouse,
            kind_label: "Mouse".to_string(),
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
            level: None,
            pages: None,
            pages_partial: false,
            pages_unanswered: false,
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

    /// The look a style has when the configuration says nothing about the text colour: the level's
    /// own colours, which is the default.
    fn look(style: TrayStyle) -> TrayLook {
        TrayLook {
            style,
            text_color: TrayTextColor::Level,
        }
    }

    /// The percentage style with a given text colour, as a bare pixmap: what the colour tests need
    /// and nothing else.
    fn text_pixmap(color: TrayTextColor) -> Icon {
        glyph(
            TrayLook {
                style: TrayStyle::Percent,
                text_color: color,
            },
            Some(65),
            false,
            false,
        )
    }

    fn tray(level: Option<u8>, style: TrayStyle) -> EmbrTray {
        let (actions, _) = std::sync::mpsc::channel();
        let mut tray = EmbrTray::new("embr".to_string(), actions);
        let icon = TrayIcon {
            key: None,
            title: "Embr".to_string(),
            detail: String::new(),
            level,
            charging: false,
            faulty: false,
        };
        tray.observe(&icon, &[], look(style));
        tray
    }

    #[test]
    fn every_percentage_is_drawn_and_stays_inside_the_icon() {
        for percent in 0..=100u8 {
            let icon = percent_icon(percent, TrayTextColor::Level);
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
                opaque(&percent_icon(percent, TrayTextColor::Level)).len(),
                expected,
                "{percent} % must draw each set bit exactly once"
            );
        }
    }

    #[test]
    fn every_style_draws_an_icon_of_the_right_size() {
        for style in TrayStyle::ALL {
            for level in [None, Some(0), Some(19), Some(65), Some(100)] {
                let icon = tray(level, style).pictogram();
                assert_eq!(icon.width, ICON_SIDE, "{style:?} at {level:?}");
                assert_eq!(icon.height, ICON_SIDE);
                assert_eq!(icon.data.len(), (ICON_SIDE * ICON_SIDE * 4) as usize);
                let points = opaque(&icon);
                assert!(!points.is_empty(), "{style:?} at {level:?} drew nothing");
                assert!(
                    points
                        .iter()
                        .all(|(x, y)| (0..ICON_SIDE).contains(x) && (0..ICON_SIDE).contains(y)),
                    "{style:?} at {level:?} drew outside the icon"
                );
            }
        }
    }

    #[test]
    fn the_five_styles_are_five_different_drawings() {
        // Compared as the picker shows them, because that is what the promise is: five tiles a
        // user can tell apart. Two of the styles are the same drawing while a device is
        // discharging — `status` is `battery` plus what a level cannot say — and the status tile is
        // previewed charging, so the picker still shows five different things.
        let icons: Vec<Vec<u8>> = TrayStyle::ALL
            .iter()
            .map(|style| preview(*style, TrayTextColor::Level).data)
            .collect();
        for (index, first) in icons.iter().enumerate() {
            for (other, second) in icons.iter().enumerate().skip(index + 1) {
                assert_ne!(
                    first,
                    second,
                    "{:?} and {:?} draw the same thing: a picker of five options would have one",
                    TrayStyle::ALL[index],
                    TrayStyle::ALL[other]
                );
            }
        }
    }

    #[test]
    fn the_style_decides_the_shape() {
        let battery = tray(Some(7), TrayStyle::Battery).pictogram();
        assert_eq!(battery.data, battery_icon(Some(7), false).data);

        let number = tray(Some(7), TrayStyle::Percent).pictogram();
        assert_eq!(number.data, percent_icon(7, TrayTextColor::Level).data);
        assert_ne!(number.data, battery.data);

        let upright = tray(Some(7), TrayStyle::BatteryVertical).pictogram();
        assert_eq!(upright.data, battery_icon(Some(7), true).data);
        assert_ne!(
            upright.data, battery.data,
            "an upright battery is not the flat one again"
        );
    }

    #[test]
    fn without_a_reading_every_style_draws_the_empty_box() {
        // There is no colour to say "no reading" with any more, so the *shape* says it: the template
        // with nothing in it. The upright style keeps its own orientation; the three with no shape
        // of their own — a number, an arc, a bolt — draw the flat box.
        assert_eq!(
            tray(None, TrayStyle::Battery).pictogram().data,
            battery_icon(None, false).data
        );
        assert_eq!(
            tray(None, TrayStyle::BatteryVertical).pictogram().data,
            battery_icon(None, true).data
        );
        for style in [TrayStyle::Percent, TrayStyle::Ring, TrayStyle::Status] {
            assert_eq!(
                tray(None, style).pictogram().data,
                battery_icon(None, false).data,
                "{style:?} has nothing to read and says so with an empty box"
            );
        }
    }

    #[test]
    fn the_glyphs_are_line_drawings_with_no_colour_of_their_own() {
        // The point of the template: the shapes carry the level, and the colour belongs to the
        // percentage style's text. A green, amber or red pixel in a glyph would be a style that
        // shouts where it should not.
        for style in [
            TrayStyle::Battery,
            TrayStyle::BatteryVertical,
            TrayStyle::Ring,
            TrayStyle::Status,
        ] {
            for level in [Some(12u8), Some(45), Some(95)] {
                let icon = tray(level, style).pictogram();
                for (x, y) in opaque(&icon) {
                    let offset = ((y * ICON_SIDE + x) * 4) as usize;
                    let pixel = &icon.data[offset + 1..offset + 4];
                    // White and dim grey both have r < g < b, and a blend of the two keeps that
                    // order; every colour this app uses for a level breaks it (green has g > b,
                    // amber and red have r > g). So the order is the test.
                    assert!(
                        pixel[0] <= pixel[1] && pixel[1] <= pixel[2],
                        "{style:?} at {level:?} drew {pixel:?} at ({x}, {y}): a glyph is white or \
                         dim, and a blend of them, never a level colour"
                    );
                }
            }
        }
    }

    #[test]
    fn the_text_takes_the_colour_the_user_chose() {
        let white = text_pixmap(TrayTextColor::White);
        let blue = text_pixmap(TrayTextColor::Blue);
        assert_ne!(white.data, blue.data, "two colours are two drawings");
        assert_eq!(white.data, percent_icon(65, TrayTextColor::White).data);
        let expected = TrayTextColor::Blue.rgb();
        let mut painted = Vec::new();
        for (x, y) in opaque(&text_pixmap(TrayTextColor::Blue)) {
            let offset = ((y * ICON_SIDE + x) * 4) as usize;
            let white = opaque(&text_pixmap(TrayTextColor::White));
            if white.contains(&(x, y)) {
                painted
                    .push(text_pixmap(TrayTextColor::Blue).data[offset + 1..offset + 4].to_vec());
            }
        }
        assert!(!painted.is_empty(), "the digits are drawn in some colour");
        assert!(
            painted
                .iter()
                .all(|pixel| pixel.as_slice() == [expected.0, expected.1, expected.2]),
            "blue means blue: {:?}",
            painted.iter().take(3).collect::<Vec<_>>()
        );

        // A fixed colour does not move with the level; the level's own does.
        assert_eq!(
            TrayTextColor::White.color(Some(5)),
            TrayTextColor::White.color(Some(95)),
            "one colour, whatever the level"
        );
        assert_ne!(
            TrayTextColor::Level.color(Some(5)),
            TrayTextColor::Level.color(Some(95)),
            "and by-level is not one colour"
        );
    }

    #[test]
    fn the_style_can_be_changed_without_a_reading() {
        // This is what the picker does, on the UI thread: the next snapshot does not have to
        // arrive for the icon to change shape, and it does not need a new level either.
        let mut item = tray(Some(64), TrayStyle::Battery);
        assert_eq!(item.pictogram().data, battery_icon(Some(64), false).data);
        item.set_look(look(TrayStyle::Percent));
        assert_eq!(
            item.pictogram().data,
            percent_icon(64, TrayTextColor::Level).data
        );
        item.set_look(look(TrayStyle::Ring));
        assert_eq!(item.pictogram().data, ring_icon(64).data);
    }

    #[test]
    fn the_ring_fills_as_the_charge_rises() {
        // The ring's empty part is grey and its arc is white, so the level *is* how much of the
        // ring is white: a shorter arc of it the lower the charge.
        let arc_pixels = |percent: u8| {
            let icon = ring_icon(percent);
            opaque(&icon)
                .iter()
                .filter(|(x, y)| {
                    let offset = ((y * ICON_SIDE + x) * 4) as usize;
                    let pixel = &icon.data[offset..offset + 4];
                    pixel[1] == WHITE.0 && pixel[2] == WHITE.1 && pixel[3] == WHITE.2
                })
                .count()
        };
        let low = arc_pixels(35);
        let middle = arc_pixels(60);
        let high = arc_pixels(100);
        assert!(
            low < middle && middle < high,
            "the arc must grow with the charge: 35 % {low}, 60 % {middle}, 100 % {high}"
        );
    }

    #[test]
    fn a_charge_and_a_fault_are_what_the_status_style_adds() {
        let plain = status_icon(Some(65), false, false);
        let charging = status_icon(Some(65), true, false);
        let broken = status_icon(Some(65), false, true);
        assert_ne!(
            plain.data, charging.data,
            "a device on external power draws something else"
        );
        assert_ne!(plain.data, broken.data, "and so does a broken battery");
        assert_ne!(charging.data, broken.data);

        // The bolt replaces the fill, so a charging icon has an empty box apart from it — while
        // the plain one is filled from the left.
        assert_eq!(
            status_icon(None, false, false).data,
            battery_icon(None, false).data,
            "with nothing to read and nothing to say, an empty box is this app's word for it"
        );
        assert_ne!(
            status_icon(None, true, false).data,
            battery_icon(None, false).data,
            "a bolt is drawn even without a level: being on power is not about the level"
        );
        assert_ne!(
            status_icon(None, false, true).data,
            battery_icon(None, false).data,
            "and so is a fault"
        );
    }

    #[test]
    fn the_picker_preview_is_the_icon_the_bar_draws() {
        // The settings page shows these; a preview drawn by a second code path could show a style
        // the tray does not draw, which is the one thing a picker must never do.
        let text = TrayTextColor::Level;
        assert_eq!(
            preview(TrayStyle::Battery, text).data,
            battery_icon(Some(35), false).data
        );
        assert_eq!(
            preview(TrayStyle::Percent, text).data,
            percent_icon(65, text).data
        );
        assert_eq!(preview(TrayStyle::Ring, text).data, ring_icon(35).data);
        assert_eq!(
            preview(TrayStyle::Status, text).data,
            status_icon(Some(65), true, false).data,
            "the status style is previewed charging: that is the thing it does that others do not"
        );
        assert_eq!(
            preview(TrayStyle::BatteryVertical, text).data,
            battery_icon(Some(35), true).data
        );
        // The percentage tile previews in the colour the user chose, so the two pickers agree.
        assert_eq!(
            preview(TrayStyle::Percent, TrayTextColor::Red).data,
            percent_icon(65, TrayTextColor::Red).data
        );
    }

    #[test]
    fn each_icon_draws_the_device_it_is_about() {
        let (actions, _) = std::sync::mpsc::channel();
        let mut item = EmbrTray::new("embr-hidraw8".to_string(), actions);

        let snapshot = snapshot(vec![
            view("hidraw7", Some(92)),
            view("hidraw8", Some(19)),
            view("hidraw0", None),
        ]);
        let icons = embr_core::tray_icons(&snapshot, &["hidraw8".to_string()]);
        item.observe(&icons[0], &labels(&snapshot), look(TrayStyle::Percent));

        assert_eq!(item.id, "embr-hidraw8", "the id is what a bar pins");
        assert_eq!(
            item.pictogram().data,
            percent_icon(19, TrayTextColor::Level).data,
            "this icon is the mouse, not the worst device on the machine"
        );
        assert!(
            item.tooltip.contains("G703 hidraw8"),
            "the tooltip names it: {}",
            item.tooltip
        );
    }

    #[test]
    fn the_default_icon_still_follows_the_worst_device() {
        let (actions, _) = std::sync::mpsc::channel();
        let mut item = EmbrTray::new("embr".to_string(), actions);

        let snapshot = snapshot(vec![view("hidraw7", Some(88)), view("hidraw8", Some(19))]);
        let icons = embr_core::tray_icons(&snapshot, &[]);
        item.observe(&icons[0], &labels(&snapshot), look(TrayStyle::Percent));

        assert_eq!(item.id, "embr");
        assert_eq!(
            item.pictogram().data,
            percent_icon(19, TrayTextColor::Level).data
        );
    }
}
