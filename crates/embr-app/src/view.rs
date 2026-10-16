//! `core`'s view model in the shape the `.slint` file wants.
//!
//! This is pure translation: the percentage text (with the `~` that marks an estimate), the
//! status label and the age all come from `core`. The only thing decided here is the one
//! thing Slint cannot express — a missing number is `-1` rather than `Option`.

use std::time::SystemTime;

use embr_core::{DeviceView, TrayLook, TrayStyle, TrayTextColor, human_age};

use crate::{Device, TrayChoice, TrayColorOption, TrayStyleOption};

/// A stand-in for "nothing selected": Slint has no `Option<Device>` for a property.
pub fn nothing() -> Device {
    Device {
        key: "".into(),
        name: "".into(),
        kind: "".into(),
        connection: "".into(),
        percent: -1,
        percent_text: "—".into(),
        has_percent: false,
        millivolts: 0,
        status: "".into(),
        attention: false,
        updated: "never".into(),
        alert_below: 20,
        alert_below_custom: false,
        error: "".into(),
    }
}

pub fn device(view: &DeviceView) -> Device {
    Device {
        key: view.key.clone().into(),
        name: view.name.clone().into(),
        kind: view.kind.clone().into(),
        connection: view.connection.clone().into(),
        percent: view.percent.map(i32::from).unwrap_or(-1),
        percent_text: view.percent_text.clone().into(),
        has_percent: view.percent.is_some(),
        millivolts: view.millivolts.map(i32::from).unwrap_or(0),
        status: view.status.clone().into(),
        attention: view.needs_attention(),
        updated: age(view).into(),
        alert_below: i32::from(view.alert_below_effective),
        alert_below_custom: view.alert_below.is_some(),
        error: view.error.clone().unwrap_or_default().into(),
    }
}

/// One device, as the settings page offers it: a switch that gives it a tray icon of its own.
///
/// The switch reads its state from the configuration rather than from the widget, so a device that
/// was chosen in an earlier run comes back chosen, and the row is rebuilt from the snapshot every
/// time anyway.
pub fn tray_choice(view: &DeviceView, chosen: &[String]) -> TrayChoice {
    TrayChoice {
        key: view.key.clone().into(),
        name: view.name.clone().into(),
        caption: view.connection.clone().into(),
        on: chosen.iter().any(|key| key == &view.key),
    }
}

/// One option on the tray-style picker: the style, and the pictogram a bar would draw for it.
///
/// The preview is the *pixmap* — through `tray::glyph`, the same function the live icon goes
/// through — and not a second drawing in the `.slint` file, so the picker cannot offer a style
/// that looks like nothing the tray draws.
pub fn tray_style_option(style: TrayStyle, look: TrayLook) -> TrayStyleOption {
    TrayStyleOption {
        id: style.as_str().into(),
        label: style.label().into(),
        caption: style.caption().into(),
        // The percentage tile is previewed with the colour the user chose, so the two pickers
        // below each other agree about what the icon will look like.
        preview: crate::tray::image(&crate::tray::preview(style, look.text_color)),
        on: style == look.style,
    }
}

/// One option on the text-colour picker: the colour, and a swatch of it.
pub fn tray_color_option(color: TrayTextColor, chosen: TrayTextColor) -> TrayColorOption {
    let (r, g, b) = color.rgb();
    TrayColorOption {
        id: color.as_str().into(),
        label: color.label().into(),
        swatch: slint::Color::from_rgb_u8(r, g, b),
        on: color == chosen,
    }
}

/// `Ring — a circle that fills as the charge rises`: what the picker's selection is, in words, for
/// the line under it.
pub fn tray_style_caption(chosen: TrayStyle) -> String {
    format!("{} — {}.", chosen.label(), chosen.caption())
}

/// `12s ago`, or `never` for a device that has never answered.
pub fn age(view: &DeviceView) -> String {
    view.captured
        .map(|captured| human_age(captured, SystemTime::now()))
        .unwrap_or_else(|| "never".to_string())
}
