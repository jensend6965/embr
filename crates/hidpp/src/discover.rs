//! "What Logitech devices are attached, how are they connected, and how full are they?"
//!
//! One call — [`discover`] — walks sysfs, opens the HID++ channel of every receiver it
//! finds, reads the pairing table, and asks each paired device and each directly attached
//! device for its battery. Failures are reported per device instead of being flattened into
//! a missing row, because "the keyboard is asleep" and "the keyboard has no battery" look
//! identical from the outside and mean very different things to a user.

use std::path::PathBuf;
use std::time::Duration;

use crate::battery::BatteryReading;
use crate::decode;
use crate::receiver::{self, Slot};
use crate::report::{DEVICE_SELF, Feature};
use crate::sysfs::{self, Node, Role};
use crate::transport::{Channel, DEVICE_TIMEOUT, RECEIVER_TIMEOUT};
use crate::{Error, Result};

/// How long a directly attached device gets to prove it speaks HID++.
///
/// A device on a cable answers in milliseconds; one that stays silent is asleep (or has no
/// battery feature at all, like a headset), and spending the full [`DEVICE_TIMEOUT`] on it
/// would delay every sweep — and therefore the first data the window shows — by seconds.
const DIRECT_PING_TIMEOUT: Duration = Duration::from_millis(700);

/// What kind of peripheral this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Mouse,
    Keyboard,
    Headset,
    Trackball,
    Touchpad,
    Numpad,
    Presenter,
    Remote,
    Unknown,
}

impl DeviceKind {
    /// Decode the device-type byte of the `DEVICE NAME {0005}` feature.
    pub fn from_hidpp(value: u8) -> Self {
        match value & 0x0F {
            0 => Self::Keyboard,
            1 => Self::Remote,
            2 => Self::Numpad,
            3 => Self::Mouse,
            4 => Self::Touchpad,
            5 => Self::Trackball,
            6 => Self::Presenter,
            7 => Self::Headset,
            _ => Self::Unknown,
        }
    }

    /// Guess from a product name; used when the device does not report its kind.
    pub fn from_name(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        if lower.contains("mouse") || lower.contains("lightspeed") && lower.contains("g7") {
            Self::Mouse
        } else if lower.contains("keyboard") || lower.contains("tkl") || lower.contains("keys") {
            Self::Keyboard
        } else if lower.contains("headset")
            || lower.contains("zone")
            || lower.contains("g733")
            || lower.contains("g pro x")
        {
            Self::Headset
        } else if lower.contains("trackball") || lower.contains("ergo") {
            Self::Trackball
        } else if lower.contains("touchpad") || lower.contains("touch pad") {
            Self::Touchpad
        } else if lower.contains("numpad") || lower.contains("number pad") {
            Self::Numpad
        } else if lower.contains("presenter") || lower.contains("spotlight") {
            Self::Presenter
        } else {
            Self::Unknown
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Mouse => "Mouse",
            Self::Keyboard => "Keyboard",
            Self::Headset => "Headset",
            Self::Trackball => "Trackball",
            Self::Touchpad => "Touchpad",
            Self::Numpad => "Numpad",
            Self::Presenter => "Presenter",
            Self::Remote => "Remote",
            Self::Unknown => "Device",
        }
    }
}

/// How a device is attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    /// Through a receiver: its family name, USB product id and slot number.
    Receiver {
        kind: &'static str,
        product: u16,
        slot: u8,
    },
    Usb,
    Bluetooth,
    Unknown,
}

impl Connection {
    pub fn label(&self) -> String {
        match self {
            Self::Receiver {
                kind,
                product,
                slot,
            } => format!("{kind} ({product:04X}, slot {slot})"),
            Self::Usb => "USB".to_string(),
            Self::Bluetooth => "Bluetooth".to_string(),
            Self::Unknown => "Unknown".to_string(),
        }
    }
}

/// One device, as far as we could determine.
#[derive(Debug, Clone)]
pub struct Device {
    /// Stable identity across runs: the serial when the device has one, else the node path.
    pub key: String,
    pub name: String,
    pub kind: DeviceKind,
    pub connection: Connection,
    /// The device's own hidraw node, when the kernel exposes one.
    pub hidraw: Option<PathBuf>,
    pub serial: Option<String>,
    pub wpid: Option<u16>,
    pub battery: Option<BatteryReading>,
    /// Why there is no battery reading, if there is none.
    pub error: Option<String>,
    /// Whether the device itself answered a HID++ call.
    ///
    /// This is what separates "present but with nothing to say about its battery" from "not
    /// there": the headset on this machine answers every other feature in 2 ms and only refuses
    /// the battery call while it is idle, so calling that state "Unavailable" reads as "the
    /// device is gone" when it is sitting on the desk in use.
    pub responded: bool,
}

impl Device {
    /// Battery percentage as text, with `~` for a value we inferred from voltage.
    pub fn percent_text(&self) -> String {
        match self.battery.as_ref().and_then(|b| b.percent) {
            Some(percent) => percent.to_string(),
            None => "—".to_string(),
        }
    }

    pub fn status_label(&self) -> &'static str {
        match &self.battery {
            Some(reading) => reading.status.label(),
            None if self.responded => "Connected",
            None => "Not answering",
        }
    }
}

/// Everything one discovery pass found.
#[derive(Debug, Default, Clone)]
pub struct DiscoveryReport {
    pub devices: Vec<Device>,
    pub warnings: Vec<String>,
    /// How many Logitech hidraw nodes sysfs exposed.
    pub nodes_seen: usize,
}

/// Discover every Logitech device and read its battery.
pub fn discover() -> DiscoveryReport {
    let mut report = DiscoveryReport::default();
    let nodes = sysfs::logitech_nodes();
    report.nodes_seen = nodes.len();

    // --- receivers first: they own their paired devices -------------------------------
    // One receiver presents several hidraw nodes, one per USB interface; group them by the
    // port they sit on and talk to the HID++ interface only, so the others are never
    // probed and do not produce a wall of EPIPE warnings.
    let mut groups: Vec<String> = Vec::new();
    for node in nodes.iter().filter(|n| n.role == Role::Receiver) {
        let group = sysfs::phys_group(node);
        if !groups.contains(&group) {
            groups.push(group);
        }
    }

    for group in &groups {
        let Some(channel_node) = sysfs::receiver_channel_in_group(&nodes, group) else {
            report
                .warnings
                .push(format!("no HID++ interface on {group}"));
            continue;
        };
        let kind = sysfs::receiver_kind(channel_node.product);
        let mut channel = match Channel::open(&channel_node.hidraw) {
            Ok(channel) => channel,
            Err(err) => {
                report
                    .warnings
                    .push(format!("{}: {err}", channel_node.hidraw.display()));
                continue;
            }
        };
        let (slots, mut warnings) = receiver::read_slots(&mut channel, RECEIVER_TIMEOUT);
        report.warnings.append(&mut warnings);
        if slots.is_empty() {
            report.warnings.push(format!(
                "{} ({}): no paired devices answered",
                channel_node.hidraw.display(),
                kind
            ));
        }
        for slot in slots {
            let mut device =
                device_from_slot(&slot, kind, channel_node.product, &nodes, &mut channel);
            device.hidraw = None;
            // prefer the device's own node for display, if sysfs exposes one
            if let Some(own) = own_node_for(&nodes, &slot) {
                device.hidraw = Some(own.hidraw.clone());
                if device.name.is_empty() {
                    device.name = own.name.clone();
                }
            }
            if device.name.is_empty() {
                device.name = format!("Unknown device {:#06X}", slot.wpid.unwrap_or(0));
            }
            if device.kind == DeviceKind::Unknown {
                device.kind = DeviceKind::from_name(&device.name);
            }
            report.devices.push(device);
        }
    }

    // --- anything attached directly (USB cable or Bluetooth) --------------------------
    for node in nodes.iter().filter(|n| n.role == Role::Direct) {
        if !node.is_hidpp_candidate() {
            continue;
        }
        let mut device = Device {
            key: node
                .unique
                .clone()
                .unwrap_or_else(|| node.hidraw_name.clone()),
            name: node.name.clone(),
            kind: DeviceKind::from_name(&node.name),
            connection: Connection::Usb,
            hidraw: Some(node.hidraw.clone()),
            serial: node.unique.clone(),
            wpid: None,
            battery: None,
            error: None,
            responded: false,
        };
        match Channel::open(&node.hidraw) {
            Ok(mut channel) => {
                // A short ping first, on ROOT: every HID++ 2.0 device implements it, so this
                // asks "are you there", not "do you happen to have a voltage feature". Waiting
                // the full device timeout for something that never answers is what turns a
                // refresh into a twenty-second stall, and a device that cannot answer in under
                // a second is asleep, not slow.
                match feature_index(&mut channel, DEVICE_SELF, Feature::Root, DIRECT_PING_TIMEOUT)
                {
                    Ok(_) => {
                        device.responded = true;
                        match read_battery(&mut channel, DEVICE_SELF, DEVICE_TIMEOUT) {
                            Ok(reading) => device.battery = Some(reading),
                            Err(err) => device.error = Some(err.to_string()),
                        }
                    }
                    Err(err) => {
                        device.error = Some(format!(
                            "no HID++ answer within {} ms ({err})",
                            DIRECT_PING_TIMEOUT.as_millis()
                        ));
                    }
                }
            }
            Err(err) => device.error = Some(err.to_string()),
        }
        report.devices.push(device);
    }

    report
}

fn device_from_slot(
    slot: &Slot,
    receiver_kind: &'static str,
    product: u16,
    nodes: &[Node],
    channel: &mut Channel,
) -> Device {
    let key = slot
        .serial
        .clone()
        .unwrap_or_else(|| format!("slot{}:{:04X}", slot.slot, slot.wpid.unwrap_or(0)));
    let mut device = Device {
        key,
        name: slot.name.clone().unwrap_or_default(),
        kind: DeviceKind::Unknown,
        connection: Connection::Receiver {
            kind: receiver_kind,
            product,
            slot: slot.slot,
        },
        hidraw: None,
        serial: slot.serial.clone(),
        wpid: slot.wpid,
        battery: None,
        error: None,
        responded: false,
    };
    if device.name.is_empty()
        && let Some(own) = own_node_for(nodes, slot)
    {
        device.name = own.name.clone();
    }
    if let Some((name, kind)) = device_name(channel, slot.slot, RECEIVER_TIMEOUT) {
        // The device answered us by name, so it is there even if its battery call fails.
        device.responded = true;
        if !name.is_empty() {
            device.name = name;
        }
        if let Some(kind) = kind {
            device.kind = kind;
        }
    }
    match read_battery(channel, slot.slot, DEVICE_TIMEOUT) {
        Ok(reading) => {
            device.responded = true;
            device.battery = Some(reading);
        }
        Err(err) => device.error = Some(err.to_string()),
    }
    device
}

/// The sysfs node belonging to a paired device, matched by serial.
fn own_node_for<'a>(nodes: &'a [Node], slot: &Slot) -> Option<&'a Node> {
    let serial = slot.serial.as_ref()?;
    nodes
        .iter()
        .find(|n| matches!(n.role, Role::PairedDevice { .. }) && n.unique.as_ref() == Some(serial))
}

/// The index a feature lives at on a device, or `None` when it is not supported.
pub fn feature_index(
    channel: &mut Channel,
    device: u8,
    feature: Feature,
    timeout: Duration,
) -> Result<Option<u8>> {
    let reply = channel.call_feature(device, 0x00, 0x00, &feature.id().to_be_bytes(), timeout)?;
    let payload = match reply.payload_or_error() {
        Ok(payload) => payload,
        // "this device does not have that feature" is an answer, not a failure
        Err(Error::Protocol2 { .. }) => return Ok(None),
        Err(err) => return Err(err),
    };
    match payload.first().copied() {
        None | Some(0) => Ok(None),
        Some(index) => {
            // Some firmware answers an unsupported feature with a HID++ 1.0 style error
            // payload instead of the documented marker; that is an answer, not an index.
            if let Some(code) = crate::report::looks_like_error_payload(payload) {
                return Err(Error::Protocol2 {
                    code,
                    name: crate::protocol2_error_name(code),
                });
            }
            Ok(Some(index))
        }
    }
}

/// Read whatever battery features the device implements and merge them into one reading.
///
/// A device that does not answer the first query is not going to answer the next one either,
/// and each unanswered call costs a full timeout — so the first timeout ends the attempt for
/// that device. Whatever was already read is still returned.
pub fn read_battery(
    channel: &mut Channel,
    device: u8,
    timeout: Duration,
) -> Result<BatteryReading> {
    let mut reading: Option<BatteryReading> = None;
    let mut tried: Vec<&'static str> = Vec::new();
    for feature in [
        Feature::BatteryStatus,
        Feature::UnifiedBattery,
        Feature::BatteryVoltage,
        // Headsets report their battery through the analogue measurement — the G733 answers
        // nothing on the 0x10xx battery features above.
        Feature::AdcMeasurement,
    ] {
        let index = match feature_index(channel, device, feature, timeout) {
            Ok(Some(index)) => index,
            Ok(None) => continue,
            Err(Error::Protocol2 { .. }) => continue,
            Err(Error::Timeout { .. }) => {
                return match reading {
                    Some(reading) => Ok(reading),
                    None => Err(Error::Timeout { waited: timeout }),
                };
            }
            Err(err) => return Err(err),
        };
        let reply = match channel.call_feature(device, index, 0x0, &[], timeout) {
            Ok(reply) => reply,
            Err(Error::Timeout { .. }) => match reading {
                Some(reading) => return Ok(reading),
                None => return Err(Error::Timeout { waited: timeout }),
            },
            Err(err) => return Err(err),
        };
        let payload = match reply.payload_or_error() {
            Ok(payload) => payload,
            Err(Error::Protocol2 { .. }) => continue,
            Err(err) => return Err(err),
        };
        let decoded = match feature {
            Feature::BatteryStatus => decode::battery_status(payload),
            Feature::UnifiedBattery => decode::unified_battery(payload),
            Feature::BatteryVoltage => decode::battery_voltage(payload),
            Feature::AdcMeasurement => decode::adc_measurement(payload),
            _ => None,
        };
        if let Some(new) = decoded {
            tried.push(feature.name());
            reading = Some(match reading {
                Some(old) => decode::merge(old, new),
                None => new,
            });
        }
    }
    match reading {
        Some(reading) => Ok(reading),
        None => Err(Error::Malformed(format!(
            "no battery feature answered (device {device:#04X}){}",
            if tried.is_empty() {
                String::new()
            } else {
                format!(" — tried {tried:?}")
            }
        ))),
    }
}

/// The device's own name and kind, from `DEVICE NAME {0005}`.
///
/// The feature splits into three functions, which the wire capture on real hardware
/// confirms: function 0 returns the name's length, function 1 returns a 16-byte fragment of
/// the name starting at a byte offset (so a long product name takes several calls), and
/// function 2 returns the device-type byte. That byte is authoritative — the string
/// heuristics in [`DeviceKind::from_name`] are only a fallback for devices that do not
/// implement the feature.
pub fn device_name(
    channel: &mut Channel,
    device: u8,
    timeout: Duration,
) -> Option<(String, Option<DeviceKind>)> {
    let index = feature_index(channel, device, Feature::DeviceName, timeout)
        .ok()
        .flatten()?;

    let reply = channel
        .call_feature(device, index, 0x0, &[], timeout)
        .ok()?;
    let length = usize::from(*reply.payload_or_error().ok()?.first()?);
    if length == 0 {
        return None;
    }
    let mut name = Vec::with_capacity(length);
    // A long report carries 16 payload bytes, so a 59-character name takes four calls;
    // the loop is bounded by the length the device itself declared.
    while name.len() < length {
        let offset = name.len().min(u8::MAX as usize) as u8;
        let reply = channel
            .call_feature(device, index, 0x1, &[offset], timeout)
            .ok()?;
        let fragment = reply.payload_or_error().ok()?;
        if fragment.is_empty() {
            break;
        }
        let room = length - name.len();
        name.extend_from_slice(&fragment[..fragment.len().min(room)]);
    }
    if name.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&name).trim().to_string();

    let kind = channel
        .call_feature(device, index, 0x2, &[], timeout)
        .ok()
        .and_then(|reply| {
            reply
                .payload_or_error()
                .ok()
                .and_then(|payload| payload.first().copied())
        })
        .map(DeviceKind::from_hidpp)
        .filter(|kind| *kind != DeviceKind::Unknown);

    Some((text, kind))
}

/// Every feature a device advertises, via `FEATURE SET {0001}`.
pub fn capabilities(channel: &mut Channel, device: u8, timeout: Duration) -> Vec<(Feature, u8)> {
    let mut found = Vec::new();
    let Ok(Some(index)) = feature_index(channel, device, Feature::FeatureSet, timeout) else {
        return found;
    };
    let Ok(reply) = channel.call_feature(device, index, 0x0, &[], timeout) else {
        return found;
    };
    let Ok(payload) = reply.payload_or_error() else {
        return found;
    };
    let Some(count) = payload.first().copied() else {
        return found;
    };
    for slot in 0..count {
        let Ok(reply) = channel.call_feature(device, index, 0x1, &[slot], timeout) else {
            continue;
        };
        let Ok(payload) = reply.payload_or_error() else {
            continue;
        };
        if payload.len() < 3 {
            continue;
        }
        let id = u16::from_be_bytes([payload[0], payload[1]]);
        if let Some(feature) = Some(id).and_then(known_feature) {
            found.push((feature, payload[2]));
        }
    }
    found
}

fn known_feature(id: u16) -> Option<Feature> {
    [
        Feature::Root,
        Feature::FeatureSet,
        Feature::DeviceFirmwareVersion,
        Feature::DeviceName,
        Feature::ConfigChange,
        Feature::BatteryStatus,
        Feature::BatteryVoltage,
        Feature::UnifiedBattery,
        Feature::WirelessDeviceStatus,
        Feature::ChangeHost,
        Feature::SpecialKeys,
        Feature::AdjustableDpi,
        Feature::RgbEffects,
        Feature::OnboardProfiles,
    ]
    .into_iter()
    .find(|f| f.id() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_decode_from_the_device_byte_and_from_names() {
        assert_eq!(DeviceKind::from_hidpp(3), DeviceKind::Mouse);
        assert_eq!(DeviceKind::from_hidpp(0), DeviceKind::Keyboard);
        assert_eq!(DeviceKind::from_hidpp(7), DeviceKind::Headset);
        assert_eq!(DeviceKind::from_hidpp(0x0F), DeviceKind::Unknown);
        assert_eq!(
            DeviceKind::from_name("G703 LIGHTSPEED Wireless Gaming Mouse w/ HERO"),
            DeviceKind::Mouse
        );
        assert_eq!(
            DeviceKind::from_name("G915 TKL LIGHTSPEED Wireless RGB Mechanical Gaming Keyboard"),
            DeviceKind::Keyboard
        );
        assert_eq!(
            DeviceKind::from_name("Logitech G733 Gaming Headset"),
            DeviceKind::Headset
        );
        assert_eq!(
            DeviceKind::from_name("Logitech USB Receiver"),
            DeviceKind::Unknown
        );
    }

    #[test]
    fn connection_labels_are_readable() {
        let receiver = Connection::Receiver {
            kind: "Lightspeed",
            product: 0xC539,
            slot: 1,
        };
        assert_eq!(receiver.label(), "Lightspeed (C539, slot 1)");
        assert_eq!(Connection::Usb.label(), "USB");
    }

    #[test]
    fn a_device_without_a_reading_says_so_instead_of_inventing_a_number() {
        let mut device = Device {
            key: "x".into(),
            name: "Logitech G733 Gaming Headset".into(),
            kind: DeviceKind::Headset,
            connection: Connection::Usb,
            hidraw: None,
            serial: None,
            wpid: None,
            battery: None,
            error: Some("no battery feature answered (device 0xFF)".into()),
            responded: false,
        };
        assert_eq!(device.percent_text(), "—");
        // Nothing answered: asleep or unplugged, and that is what the row has to say.
        assert_eq!(device.status_label(), "Not answering");
        // It answered us and simply has no battery reading to give — the state the idle headset
        // is in. Calling that "Unavailable" reads as "gone", which it is not.
        device.responded = true;
        assert_eq!(device.status_label(), "Connected");
    }

    #[test]
    fn feature_ids_are_recognised() {
        assert_eq!(known_feature(0x1001), Some(Feature::BatteryVoltage));
        assert_eq!(known_feature(0x1234), None);
    }
}
