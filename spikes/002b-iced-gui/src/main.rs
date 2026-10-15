//! Spike 002b — the "Devices" main window of a Solaar rewrite, prototyped in
//! **iced 0.14** (pure Rust, Elm architecture, wgpu renderer, no webview).
//!
//! The goal is to judge look + ergonomics, so every value is hardcoded and
//! believable instead of being read from real hardware.

use iced::font::{Family, Weight};
use iced::theme::Palette;
use iced::widget::{
    button, column, container, progress_bar, row, scrollable, text, toggler, Space,
};
use iced::{
    alignment::Horizontal, window, Alignment, Background, Border, Color, Element, Font, Length,
    Shadow, Size, Subscription, Theme, Vector,
};

// ------------------------------------------------------------------ theme ---

const BG: Color = Color::from_rgb8(0x0e, 0x10, 0x14);
const SURFACE: Color = Color::from_rgb8(0x17, 0x1a, 0x20);
const SURFACE_HOVER: Color = Color::from_rgb8(0x1e, 0x22, 0x2a);
const SURFACE_SELECTED: Color = Color::from_rgb8(0x21, 0x27, 0x33);
const LINE: Color = Color::from_rgb8(0x27, 0x2c, 0x36);
const TEXT: Color = Color::from_rgb8(0xe9, 0xec, 0xf1);
const MUTED: Color = Color::from_rgb8(0x93, 0x9c, 0xab);
const DIMMER: Color = Color::from_rgb8(0x69, 0x72, 0x82);
const ACCENT: Color = Color::from_rgb8(0x5b, 0x9d, 0xf6);
const VIOLET: Color = Color::from_rgb8(0xa9, 0x8a, 0xf0);
const GREEN: Color = Color::from_rgb8(0x4c, 0xd1, 0x8f);
const AMBER: Color = Color::from_rgb8(0xf5, 0xb5, 0x4a);
const RED: Color = Color::from_rgb8(0xf0, 0x6b, 0x6b);

const BOLD: Font = Font { weight: Weight::Bold, ..Font::DEFAULT };
const SEMIBOLD: Font = Font { weight: Weight::Semibold, ..Font::DEFAULT };
const MONO: Font = Font { family: Family::Monospace, ..Font::DEFAULT };

/// The same color at a different opacity — iced has no `with_alpha` helper.
fn fade(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

// ------------------------------------------------------------------- data ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Connection {
    Bolt,
    Lightspeed,
    Usb,
}

impl Connection {
    fn label(self) -> &'static str {
        match self {
            Connection::Bolt => "Bolt",
            Connection::Lightspeed => "Lightspeed",
            Connection::Usb => "USB",
        }
    }

    fn tint(self) -> Color {
        match self {
            Connection::Bolt => ACCENT,
            Connection::Lightspeed => GREEN,
            Connection::Usb => VIOLET,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    fn tint(self) -> Color {
        match self {
            Status::Discharging => MUTED,
            Status::Charging => GREEN,
            Status::Offline => RED,
        }
    }
}

struct Device {
    name: &'static str,
    kind: &'static str,
    connection: Connection,
    /// `None` = the device is not currently reachable.
    level: Option<f32>,
    millivolts: &'static str,
    status: Status,
    icon: &'static str,
    updated: &'static str,
}

impl Device {
    fn level_tint(level: Option<f32>) -> Color {
        match level {
            Some(v) if v <= 10.0 => RED,
            Some(v) if v <= 30.0 => AMBER,
            Some(_) => GREEN,
            None => DIMMER,
        }
    }

    fn level_label(level: Option<f32>) -> String {
        match level {
            Some(v) => format!("{v:.0}%"),
            None => "—".to_string(),
        }
    }
}

// ------------------------------------------------------------------- app ----

#[derive(Debug, Clone)]
enum Message {
    Select(usize),
    ToggleBatteryNotifications(bool),
    /// Reports the size the compositor actually gave the window.
    Resized(Size),
}

struct App {
    devices: Vec<Device>,
    selected: usize,
    battery_notifications: bool,
}

impl App {
    fn new() -> Self {
        // Icon placeholder: ASCII letters by default; `SPIKE_GLYPHS=symbols`
        // swaps in Unicode/emoji glyphs to probe font coverage.
        let icons: [&'static str; 3] = if std::env::var("SPIKE_GLYPHS").as_deref() == Ok("symbols")
        {
            ["🖱", "⌨", "🎧"]
        } else {
            ["M", "K", "H"]
        };

        Self {
            devices: vec![
                Device {
                    name: "G703 LIGHTSPEED",
                    kind: "Mouse",
                    connection: Connection::Lightspeed,
                    level: Some(88.0),
                    millivolts: "3 980 mV",
                    status: Status::Discharging,
                    icon: icons[0],
                    updated: "12 s ago",
                },
                Device {
                    name: "G915 TKL",
                    kind: "Keyboard",
                    connection: Connection::Bolt,
                    level: Some(95.0),
                    millivolts: "4 120 mV",
                    status: Status::Discharging,
                    icon: icons[1],
                    updated: "34 s ago",
                },
                Device {
                    name: "G733",
                    kind: "Headset",
                    connection: Connection::Usb,
                    level: None,
                    millivolts: "—",
                    status: Status::Offline,
                    icon: icons[2],
                    updated: "3 h ago",
                },
            ],
            selected: 0,
            battery_notifications: true,
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Select(index) => self.selected = index,
            Message::ToggleBatteryNotifications(enabled) => {
                self.battery_notifications = enabled;
            }
            Message::Resized(size) => {
                println!("[spike] window size = {}x{}", size.width, size.height);
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        window::resize_events().map(|(_id, size)| Message::Resized(size))
    }

    fn theme(&self) -> Theme {
        Theme::custom(
            "Solaar Dark",
            Palette {
                background: BG,
                text: TEXT,
                primary: ACCENT,
                success: GREEN,
                warning: AMBER,
                danger: RED,
            },
        )
    }

    fn view(&self) -> Element<'_, Message> {
        let body = row![
            container(device_list(self)).width(Length::FillPortion(3)),
            details_card(&self.devices[self.selected], self.battery_notifications),
        ]
        .spacing(14)
        .height(Length::Fill);

        let page = column![header(), divider(), body]
            .spacing(14)
            .height(Length::Fill);

        container(page)
            .padding(20)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(Background::Color(BG)),
                ..container::Style::default()
            })
            .into()
    }
}

fn main() -> iced::Result {
    // This desktop tiles new windows, so for the "intended window size" shot we
    // can pin the size: `SPIKE_FIXED_SIZE=1` makes the window non-resizable.
    let fixed = std::env::var("SPIKE_FIXED_SIZE").as_deref() == Ok("1");

    let application = iced::application(App::new, App::update, App::view)
        .title("Devices")
        .theme(App::theme)
        .subscription(App::subscription)
        .window_size(Size::new(900.0, 560.0));

    if fixed {
        application.resizable(false).centered().run()
    } else {
        application.centered().run()
    }
}

// ----------------------------------------------------------------- pieces ---

/// A 1px separator, so the look does not depend on the theme's rule color.
fn divider() -> Element<'static, Message> {
    container(Space::new().width(Length::Fill).height(1))
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(Background::Color(LINE)),
            ..container::Style::default()
        })
        .into()
}

fn header() -> Element<'static, Message> {
    row![
        text("Solaar").size(21).font(BOLD).color(TEXT),
        Space::new().width(Length::Fill),
        text("Last updated 12:41").size(12).color(DIMMER),
    ]
    .align_y(Alignment::Center)
    .into()
}

fn device_list(app: &App) -> Element<'_, Message> {
    let rows = app
        .devices
        .iter()
        .enumerate()
        .fold(column![].spacing(8), |list, (index, device)| {
            list.push(device_row(device, index, index == app.selected))
        });

    scrollable(rows.width(Length::Fill))
        .height(Length::Fill)
        .into()
}

/// One clickable device row: icon, name + connection badge, status, battery.
fn device_row(device: &Device, index: usize, selected: bool) -> Element<'_, Message> {
    let content = row![
        icon_badge(device.icon, device.status),
        column![
            row![
                text(device.name).size(14).font(SEMIBOLD).color(TEXT),
                connection_chip(device.connection),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text(device.status.label())
                .size(11)
                .color(device.status.tint()),
        ]
        .spacing(3)
        .width(Length::Fill),
        battery_meter(device),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    button(content)
        .padding([11, 12])
        .width(Length::Fill)
        .on_press(Message::Select(index))
        .style(move |theme, status| row_style(theme, status, selected))
        .into()
}

fn row_style(_theme: &Theme, status: button::Status, selected: bool) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed if !selected => SURFACE_HOVER,
        _ if selected => SURFACE_SELECTED,
        _ => SURFACE,
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color: TEXT,
        border: Border {
            color: if selected { fade(ACCENT, 0.55) } else { LINE },
            width: 1.0,
            radius: 12.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

fn icon_badge(icon: &'static str, status: Status) -> Element<'static, Message> {
    let tint = match status {
        Status::Offline => DIMMER,
        _ => ACCENT,
    };

    container(text(icon).size(15).font(BOLD).color(tint))
        .width(Length::Fixed(38.0))
        .height(Length::Fixed(38.0))
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(Background::Color(fade(tint, 0.13))),
            border: Border {
                color: fade(tint, 0.32),
                width: 1.0,
                radius: 10.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn connection_chip(connection: Connection) -> Element<'static, Message> {
    let tint = connection.tint();

    container(text(connection.label()).size(10).font(SEMIBOLD).color(tint))
        .padding([2, 7])
        .style(move |_theme| container::Style {
            background: Some(Background::Color(fade(tint, 0.12))),
            border: Border {
                color: fade(tint, 0.30),
                width: 1.0,
                radius: 100.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn battery_meter(device: &Device) -> Element<'static, Message> {
    let tint = Device::level_tint(device.level);
    let value = device.level.unwrap_or(0.0);

    row![
        progress_bar(0.0..=100.0, value)
            .length(Length::Fill)
            .girth(6)
            .style(move |_theme| progress_bar::Style {
                background: Background::Color(LINE),
                bar: Background::Color(tint),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 3.0.into(),
                },
            }),
        text(Device::level_label(device.level))
            .size(12)
            .font(MONO)
            .color(tint)
            .width(Length::Fixed(40.0))
            .align_x(Horizontal::Right),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Length::Fixed(178.0))
    .into()
}

fn details_card(device: &Device, battery_notifications: bool) -> Element<'static, Message> {
    let tint = Device::level_tint(device.level);

    let body = column![
        text(device.name).size(17).font(BOLD).color(TEXT),
        row![
            text(format!("{} · {}", device.kind, device.connection.label()))
                .size(12)
                .color(MUTED),
            Space::new().width(Length::Fill),
            text(Device::level_label(device.level))
                .size(13)
                .font(MONO)
                .color(tint),
        ]
        .align_y(Alignment::Center),
        divider(),
        kv_row(
            "Battery",
            format!(
                "{}  ·  {}",
                Device::level_label(device.level),
                device.millivolts
            ),
        ),
        kv_row("Status", device.status.label().to_string()),
        kv_row("Last updated", device.updated.to_string()),
        divider(),
        text("Settings").size(13).font(SEMIBOLD).color(TEXT),
        setting_row(
            "Battery notifications",
            toggler(battery_notifications)
                .on_toggle(Message::ToggleBatteryNotifications)
                .size(20)
                .into(),
        ),
        setting_row(
            "Notification thresholds",
            text("20%  /  10%").size(12).font(MONO).color(TEXT).into(),
        ),
    ]
    .spacing(11);

    container(body)
        .padding(16)
        .width(Length::FillPortion(2))
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(Background::Color(SURFACE)),
            border: Border {
                color: LINE,
                width: 1.0,
                radius: 14.0.into(),
            },
            shadow: Shadow {
                color: fade(Color::BLACK, 0.22),
                offset: Vector::new(0.0, 3.0),
                blur_radius: 12.0,
            },
            ..container::Style::default()
        })
        .into()
}

fn kv_row(label: &'static str, value: String) -> Element<'static, Message> {
    row![
        text(label)
            .size(12)
            .color(MUTED)
            .width(Length::Fixed(104.0)),
        text(value).size(13).color(TEXT),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

fn setting_row(
    label: &'static str,
    control: Element<'static, Message>,
) -> Element<'static, Message> {
    row![
        text(label).size(12).color(MUTED),
        Space::new().width(Length::Fill),
        control,
    ]
    .align_y(Alignment::Center)
    .into()
}
