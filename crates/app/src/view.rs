//! `core`'s view model in the shape the `.slint` file wants.
//!
//! This is pure translation: the percentage text (with the `~` that marks an estimate), the
//! status label and the age all come from `core`. The only things decided here are the two
//! things Slint cannot express — a missing number is `-1` rather than `Option`, and a device
//! kind is a one-letter glyph.

use std::time::SystemTime;

use logi_core::{DeviceView, human_age};

use crate::Device;

/// A stand-in for "nothing selected": Slint has no `Option<Device>` for a property.
pub fn nothing() -> Device {
    Device {
        key: "".into(),
        icon: "·".into(),
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
        error: "".into(),
    }
}

pub fn device(view: &DeviceView) -> Device {
    Device {
        key: view.key.clone().into(),
        icon: glyph(&view.kind).into(),
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
        error: view.error.clone().unwrap_or_default().into(),
    }
}

/// `12s ago`, or `never` for a device that has never answered.
pub fn age(view: &DeviceView) -> String {
    view.captured
        .map(|captured| human_age(captured, SystemTime::now()))
        .unwrap_or_else(|| "never".to_string())
}

fn glyph(kind: &str) -> &'static str {
    match kind {
        "Mouse" => "M",
        "Keyboard" => "K",
        "Headset" => "H",
        "Trackball" => "T",
        "Touchpad" => "P",
        "Numpad" => "N",
        "Presenter" => "S",
        "Remote" => "R",
        _ => "?",
    }
}
