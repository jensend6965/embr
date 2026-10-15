// Spike 002c — egui/eframe implementation of the Solaar main window ("Devices").
// Hardcoded believable device data; this spike measures the toolkit, not the data layer.
//
// Written against egui/eframe 0.36, where:
//   * `eframe::App` replaced `fn update(&mut self, ctx, frame)` with `fn ui(&mut self, ui: &mut Ui, frame)`
//   * `SidePanel`/`TopBottomPanel` were unified into a single `egui::Panel`
//   * panels are shown into a `&mut Ui`, not a `&Context`

use eframe::egui;
use egui::{
    pos2, vec2, Align, Color32, CornerRadius, Layout, Margin, Rect, Response, RichText, Sense,
    Stroke, Ui, UiBuilder,
};

// ---------------------------------------------------------------- palette
const BG: Color32 = Color32::from_rgb(0x16, 0x17, 0x1b);
const CARD: Color32 = Color32::from_rgb(0x20, 0x22, 0x28);
const CARD_SEL: Color32 = Color32::from_rgb(0x2a, 0x30, 0x3d);
const HOVER: Color32 = Color32::from_rgb(0x27, 0x2a, 0x31);
const TRACK: Color32 = Color32::from_rgb(0x33, 0x36, 0x3e);
const LINE: Color32 = Color32::from_rgb(0x2e, 0x31, 0x39);
const TEXT: Color32 = Color32::from_rgb(0xe8, 0xea, 0xed);
const MUTED: Color32 = Color32::from_rgb(0x96, 0x9d, 0xa8);
const DIM: Color32 = Color32::from_rgb(0x6b, 0x72, 0x7d);
const ACCENT: Color32 = Color32::from_rgb(0x5b, 0x8d, 0xef);
const GREEN: Color32 = Color32::from_rgb(0x3f, 0xc1, 0x7a);
const AMBER: Color32 = Color32::from_rgb(0xdf, 0x9b, 0x38);
const GREY: Color32 = Color32::from_rgb(0x5a, 0x60, 0x6a);

// ---------------------------------------------------------------- model
#[derive(Clone, Copy, PartialEq)]
enum Transport {
    Bolt,
    Lightspeed,
    Usb,
}
impl Transport {
    fn label(self) -> &'static str {
        match self {
            Transport::Bolt => "Bolt",
            Transport::Lightspeed => "Lightspeed",
            Transport::Usb => "USB",
        }
    }
    fn tint(self) -> Color32 {
        match self {
            Transport::Bolt => Color32::from_rgb(0x7a, 0xb0, 0xff),
            Transport::Lightspeed => ACCENT,
            Transport::Usb => Color32::from_rgb(0x9c, 0xa6, 0xb4),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Discharging,
    Charging,
    Offline,
}
impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Discharging => "Discharging",
            Status::Charging => "Charging",
            Status::Offline => "Offline",
        }
    }
    fn color(self) -> Color32 {
        match self {
            Status::Discharging => AMBER,
            Status::Charging => GREEN,
            Status::Offline => GREY,
        }
    }
}

#[derive(Clone, Copy)]
struct Device {
    glyph: &'static str,
    name: &'static str,
    kind: &'static str,
    transport: Transport,
    status: Status,
    percent: u8,
    millivolts: u16,
    updated: &'static str,
}

fn devices() -> Vec<Device> {
    vec![
        Device {
            glyph: "\u{1F5B1}", // mouse
            name: "G703 LIGHTSPEED",
            kind: "Mouse",
            transport: Transport::Lightspeed,
            status: Status::Discharging,
            percent: 88,
            millivolts: 3850,
            updated: "just now",
        },
        Device {
            glyph: "\u{2328}", // keyboard
            name: "G915 TKL",
            kind: "Keyboard",
            transport: Transport::Lightspeed,
            status: Status::Discharging,
            percent: 95,
            millivolts: 4060,
            updated: "12 s ago",
        },
        Device {
            glyph: "\u{1F3A7}", // headphones
            name: "G733",
            kind: "Headset",
            transport: Transport::Bolt,
            status: Status::Offline,
            percent: 0,
            millivolts: 0,
            updated: "3 min ago",
        },
    ]
}

// ---------------------------------------------------------------- app
struct App {
    devices: Vec<Device>,
    selected: usize,
    notify_switch: bool,
    autostart_switch: bool,
    // spike instrumentation
    t0: std::time::Instant,
    logged: bool,
    logged_size: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, t0: std::time::Instant) -> Self {
        cc.egui_ctx
            .set_theme(egui::ThemePreference::Dark);
        cc.egui_ctx.all_styles_mut(|style| {
            style.visuals = egui::Visuals::dark();
            style.visuals.panel_fill = BG;
            style.visuals.window_fill = BG;
            style.visuals.extreme_bg_color = TRACK;
            style.visuals.faint_bg_color = CARD;
            style.visuals.override_text_color = Some(TEXT);
            style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
            style.spacing.item_spacing = vec2(8.0, 6.0);
            style.spacing.scroll.bar_width = 8.0;
        });

        Self {
            devices: devices(),
            selected: 0,
            notify_switch: true,
            autostart_switch: false,
            t0,
            logged: false,
            logged_size: false,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if !self.logged {
            self.logged = true;
            let elapsed = self.t0.elapsed().as_secs_f64() * 1000.0;
            eprintln!("SPIKE_FIRST_FRAME_MS={:.1}", elapsed);
        }
        let vp = ui.ctx().input(|i| i.viewport().clone());
        if let Some(r) = vp.inner_rect {
            if !self.logged_size {
                self.logged_size = true;
                eprintln!(
                    "SPIKE_WINDOW ppp={} inner_rect={:.0}x{:.0} outer_rect={:?}",
                    ui.ctx().pixels_per_point(),
                    r.width(),
                    r.height(),
                    vp.outer_rect.map(|o| (o.width(), o.height())),
                );
            }
        }
        self.header(ui);
        self.sidebar(ui);
        self.details(ui);
    }
}

impl App {
    fn header(&self, ui: &mut Ui) {
        egui::Panel::top("header")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(20, 12)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Devices").size(19.0).color(TEXT).strong());
                    ui.add_space(6.0);
                    ui.label(RichText::new("3 connected").size(12.0).color(DIM));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new("Last updated 12:41").size(12.0).color(MUTED));
                    });
                });
            });
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        egui::Panel::left("devices")
            .resizable(false)
            .show_separator_line(false)
            .default_size(392.0)
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(Margin {
                        left: 18,
                        right: 8,
                        top: 2,
                        bottom: 16,
                    }),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for i in 0..self.devices.len() {
                            let selected = i == self.selected;
                            if self.device_row(ui, i, selected).clicked() {
                                self.selected = i;
                            }
                            ui.add_space(8.0);
                        }
                    });
            });
    }

    fn device_row(&self, ui: &mut Ui, idx: usize, selected: bool) -> Response {
        let full_w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(full_w, 68.0), Sense::hover());
        let resp = ui.interact(rect, ui.id().with(("dev", idx)), Sense::click());

        let bg = if selected {
            CARD_SEL
        } else if resp.hovered() {
            HOVER
        } else {
            CARD
        };
        ui.painter().rect_filled(rect, CornerRadius::same(10), bg);
        if selected {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(10),
                Stroke::new(1.0, ACCENT.gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
        }

        let d = self.devices[idx];
        let inner = rect.shrink2(vec2(12.0, 10.0));
        let mut row = ui.new_child(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
        );

        // leading icon on its own rounded tile
        let icon_rect = Rect::from_center_size(
            pos2(inner.left() + 16.0, inner.center().y),
            vec2(32.0, 32.0),
        );
        row.painter()
            .rect_filled(icon_rect, CornerRadius::same(8), BG);
        row.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            d.glyph,
            egui::FontId::proportional(17.0),
            TEXT,
        );
        row.advance_cursor_after_rect(icon_rect);
        row.add_space(6.0);

        // name + connection badge + status
        row.vertical(|ui| {
            ui.add_space(3.0);
            ui.label(RichText::new(d.name).size(14.0).color(TEXT).strong());
            ui.horizontal(|ui| {
                badge(ui, d.transport.label(), d.transport.tint());
                ui.label(
                    RichText::new(d.status.label())
                        .size(11.5)
                        .color(d.status.color()),
                );
            });
        });

        // battery bar + percent, right aligned
        row.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if d.status == Status::Offline {
                ui.label(RichText::new("--").size(13.0).color(DIM));
            } else {
                ui.label(
                    RichText::new(format!("{}%", d.percent))
                        .size(13.0)
                        .color(TEXT)
                        .strong(),
                );
            }
            ui.add_space(2.0);
            let pct = if d.status == Status::Offline {
                0.0
            } else {
                d.percent as f32 / 100.0
            };
            battery_bar(ui, 96.0, pct, bar_color(d.percent, d.status));
        });

        resp
    }

    fn details(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(Margin {
                        left: 8,
                        right: 18,
                        top: 2,
                        bottom: 16,
                    }),
            )
            .show(ui, |ui| {
                let d = self.devices[self.selected];

                card(ui, |ui| {
                    // header: icon tile + name + meta
                    let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                    ui.painter()
                        .rect_filled(r, CornerRadius::same(10), BG);
                    ui.painter().text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        d.glyph,
                        egui::FontId::proportional(22.0),
                        TEXT,
                    );
                    let head = Rect::from_min_max(
                        pos2(r.right() + 12.0, r.top()),
                        pos2(ui.max_rect().right(), r.bottom()),
                    );
                    let mut info = ui.new_child(
                        UiBuilder::new()
                            .max_rect(head)
                            .layout(Layout::top_down(Align::LEFT)),
                    );
                    info.add_space(1.0);
                    info.label(RichText::new(d.name).size(17.0).color(TEXT).strong());
                    info.label(
                        RichText::new(format!(
                            "{}  ·  {}  ·  {}",
                            d.kind,
                            d.transport.label(),
                            d.status.label()
                        ))
                        .size(12.0)
                        .color(MUTED),
                    );

                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(14.0);

                    kv_row(ui, "Battery", &format!("{}%", d.percent), TEXT);
                    kv_row(ui, "Voltage", &format!("{} mV", d.millivolts), MUTED);
                    kv_row(ui, "Last updated", d.updated, MUTED);

                    ui.add_space(8.0);
                    let pct = d.percent as f32 / 100.0;
                    battery_bar(
                        ui,
                        ui.available_width() - 4.0,
                        pct,
                        bar_color(d.percent, d.status),
                    );
                });

                ui.add_space(14.0);

                card(ui, |ui| {
                    ui.label(RichText::new("Settings").size(14.0).color(TEXT).strong());
                    ui.add_space(12.0);

                    switch_row(
                        ui,
                        "Battery notifications",
                        "Warn when a device drops below a threshold".to_owned(),
                        &mut self.notify_switch,
                    );

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("Notification thresholds")
                                    .size(13.0)
                                    .color(TEXT),
                            );
                            ui.label(
                                RichText::new("Warn at these battery levels")
                                    .size(11.0)
                                    .color(DIM),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            pill(ui, "10%");
                            pill(ui, "20%");
                        });
                    });

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(12.0);

                    switch_row(
                        ui,
                        "Start on login",
                        "Launch the agent with the desktop session".to_owned(),
                        &mut self.autostart_switch,
                    );
                });
            });
    }
}

// ---------------------------------------------------------------- helpers
fn bar_color(percent: u8, status: Status) -> Color32 {
    match status {
        Status::Offline => DIM,
        Status::Charging => GREEN,
        Status::Discharging => {
            if percent <= 10 {
                Color32::from_rgb(0xe5, 0x6b, 0x6b)
            } else if percent <= 20 {
                AMBER
            } else {
                GREEN
            }
        }
    }
}

fn battery_bar(ui: &mut Ui, width: f32, frac: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(24.0), 8.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(4), TRACK);
    let f = frac.clamp(0.0, 1.0);
    if f > 0.0 {
        let mut fill = rect;
        fill.set_width((rect.width() * f).max(6.0));
        ui.painter()
            .rect_filled(fill, CornerRadius::same(4), color);
    }
}

fn badge(ui: &mut Ui, text: &str, tint: Color32) {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), egui::FontId::proportional(10.5), tint);
    let size = vec2(galley.size().x + 14.0, 17.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(4), tint.gamma_multiply(0.18));
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0, tint.gamma_multiply(0.45)),
        egui::StrokeKind::Inside,
    );
    let gp = rect.center() - galley.size() / 2.0;
    ui.painter().galley(gp, galley, tint);
}

fn pill(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(11.5),
        MUTED,
    );
    let size = vec2(galley.size().x + 18.0, 24.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(12), TRACK);
    let gp = rect.center() - galley.size() / 2.0;
    ui.painter().galley(gp, galley, MUTED);
}

fn kv_row(ui: &mut Ui, key: &str, value: &str, value_color: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).size(12.5).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).size(12.5).color(value_color));
        });
    });
    ui.add_space(7.0);
}

fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .stroke(Stroke::new(1.0, LINE))
        .show(ui, add);
}

fn switch_row(ui: &mut Ui, title: &str, sub: String, on: &mut bool) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(13.0).color(TEXT));
            ui.label(RichText::new(sub).size(11.0).color(DIM));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            switch(ui, on);
        });
    });
}

/// egui ships no switch/toggle widget; this is the hand-rolled equivalent (~15 lines).
fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let track = lerp_color(Color32::from_rgb(0x44, 0x48, 0x50), ACCENT, t);
    ui.painter()
        .rect_filled(rect, CornerRadius::same(11), track);
    let cx = egui::lerp((rect.left() + 12.0)..=(rect.right() - 12.0), t);
    ui.painter()
        .circle_filled(pos2(cx, rect.center().y), 8.5, Color32::WHITE);
    resp
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    Color32::from_rgb(
        egui::lerp(a.r() as f32..=b.r() as f32, t) as u8,
        egui::lerp(a.g() as f32..=b.g() as f32, t) as u8,
        egui::lerp(a.b() as f32..=b.b() as f32, t) as u8,
    )
}

fn main() -> eframe::Result<()> {
    let t0 = std::time::Instant::now();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Devices")
            .with_app_id("solaar-spike-egui")
            .with_inner_size([900.0, 560.0])
            .with_min_inner_size([720.0, 460.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Devices",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, t0)))),
    )
}
