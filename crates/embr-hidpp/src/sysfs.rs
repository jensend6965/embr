//! What Logitech nodes exist, and how they relate to each other.
//!
//! Everything here comes from `/sys/class/hidraw` and the `uevent` files, so it needs no
//! udev library and no privileges. The relationships are structural, which is what makes
//! them reliable:
//!
//! * a **receiver** node is bound to `logitech-djreceiver` and lives at `<usb-iface>/inputN`,
//!   where the `input2` interface is the HID++ channel;
//! * a **paired device** is bound to `logitech-hidpp-device` and its sysfs directory is
//!   nested *inside* the receiver's directory — so the link device → receiver needs no
//!   guessing;
//! * a **direct** device (USB cable, or Bluetooth) is neither of those, and answers HID++
//!   as itself.

use std::path::{Path, PathBuf};

/// Logitech's USB vendor id.
pub const LOGITECH_VENDOR: u16 = 0x046D;
/// The Lenovo nano receiver is a rebadged Logitech one and speaks the same protocol.
pub const LENOVO_VENDOR: u16 = 0x17EF;
pub const LENOVO_NANO_RECEIVER: u16 = 0x6042;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// Receiver: it speaks HID++ and holds up to six paired devices.
    Receiver,
    /// A device paired to a receiver; the path is the receiver's sysfs directory.
    PairedDevice { receiver_dir: PathBuf },
    /// Attached directly (USB or Bluetooth).
    Direct,
}

/// One Logitech hidraw node as sysfs describes it.
#[derive(Debug, Clone)]
pub struct Node {
    /// `/dev/hidrawN`
    pub hidraw: PathBuf,
    /// `hidrawN`
    pub hidraw_name: String,
    pub role: Role,
    pub vendor: u16,
    pub product: u16,
    /// `HID_NAME`, e.g. `Logitech USB Receiver` or `Logitech G703 LS`.
    pub name: String,
    /// `HID_UNIQ` normalised to upper-case hex (`34-81-81-a8` → `348181A8`), when it
    /// carries anything but zeros.
    pub unique: Option<String>,
    /// Kernel driver bound to the node.
    pub driver: String,
    /// `HID_PHYS`, e.g. `usb-0000:02:00.0-10/input2:1`.
    pub phys: String,
    /// sysfs directory of the HID device.
    pub device_dir: PathBuf,
    /// USB interface index parsed out of `HID_PHYS` (`input2` → 2).
    pub usb_interface: Option<u8>,
}

impl Node {
    /// A receiver's HID++ channel is its highest-numbered interface.
    pub fn is_hidpp_candidate(&self) -> bool {
        self.usb_interface.is_none_or(|i| i >= 2)
    }
}

/// Every Logitech (and Lenovo-receiver) hidraw node on this machine, ordered by name.
pub fn logitech_nodes() -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/hidraw") else {
        return nodes;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("hidraw") {
            continue;
        }
        let dev_dir = entry.path().join("device");
        let Ok(device_dir) = std::fs::canonicalize(&dev_dir) else {
            continue;
        };
        let Some(uevent) = read_uevent(&device_dir) else {
            continue;
        };
        let Some((vendor, product)) = parse_hid_id(uevent.get("HID_ID")) else {
            continue;
        };
        if vendor != LOGITECH_VENDOR
            && !(vendor == LENOVO_VENDOR && product == LENOVO_NANO_RECEIVER)
        {
            continue;
        }
        let driver = uevent.get("DRIVER").cloned().unwrap_or_default();
        let phys = uevent.get("HID_PHYS").cloned().unwrap_or_default();
        let role = classify(&driver, &device_dir);
        nodes.push(Node {
            hidraw: PathBuf::from(format!("/dev/{name}")),
            hidraw_name: name,
            role,
            vendor,
            product,
            name: uevent.get("HID_NAME").cloned().unwrap_or_default(),
            unique: uevent.get("HID_UNIQ").and_then(|u| normalise_unique(u)),
            driver,
            usb_interface: parse_interface(&phys),
            phys,
            device_dir,
        });
    }
    nodes.sort_by(|a, b| a.hidraw_name.cmp(&b.hidraw_name));
    nodes
}

/// A receiver exposes three hidraw nodes (one per USB interface) and only the HID++ one
/// answers; writes to the others fail with `EPIPE`. This is the key that groups them: the
/// USB port they share, e.g. `usb-0000:02:00.0-10`.
pub fn phys_group(node: &Node) -> String {
    node.phys.split('/').next().unwrap_or_default().to_string()
}

/// The node a receiver can be talked to on, preferring the `input2` HID++ interface.
pub fn receiver_channel<'a>(nodes: &'a [Node], receiver_dir: &Path) -> Option<&'a Node> {
    nodes
        .iter()
        .filter(|n| n.role == Role::Receiver && n.device_dir == receiver_dir)
        .max_by_key(|n| n.usb_interface.unwrap_or(0))
}

/// The HID++ channel of one receiver, identified by the USB port it sits on.
pub fn receiver_channel_in_group<'a>(nodes: &'a [Node], group: &str) -> Option<&'a Node> {
    nodes
        .iter()
        .filter(|n| n.role == Role::Receiver && phys_group(n) == group)
        .max_by_key(|n| n.usb_interface.unwrap_or(0))
}

fn classify(driver: &str, device_dir: &Path) -> Role {
    if driver == "logitech-djreceiver" {
        return Role::Receiver;
    }
    if driver == "logitech-hidpp-device"
        && let Some(parent) = device_dir.parent()
        && let Some(uevent) = read_uevent(parent)
        && uevent
            .get("DRIVER")
            .is_some_and(|d| d == "logitech-djreceiver")
    {
        return Role::PairedDevice {
            receiver_dir: parent.to_path_buf(),
        };
    }
    Role::Direct
}

fn read_uevent(dir: &Path) -> Option<std::collections::HashMap<String, String>> {
    let text = std::fs::read_to_string(dir.join("uevent")).ok()?;
    let mut map = std::collections::HashMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    Some(map)
}

/// `0003:0000046D:00000C547` → `(0x046D, 0xC547)`
fn parse_hid_id(value: Option<&String>) -> Option<(u16, u16)> {
    let parts: Vec<&str> = value?.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let vendor = u16::from_str_radix(parts[1].trim_start_matches('0'), 16).ok()?;
    let product = u16::from_str_radix(parts[2].trim_start_matches('0'), 16).ok()?;
    Some((vendor, product))
}

/// `usb-0000:02:00.0-10/input2:1` → `Some(2)`
fn parse_interface(phys: &str) -> Option<u8> {
    let tail = phys.rsplit('/').next()?;
    let digits: String = tail
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// `34-81-81-a8` → `348181A8`; all-zero values mean "no unique id".
fn normalise_unique(raw: &str) -> Option<String> {
    let cleaned: String = raw.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '0') {
        return None;
    }
    Some(cleaned.to_uppercase())
}

/// Human name for a receiver product id, as Logitech brands them.
pub fn receiver_kind(product: u16) -> &'static str {
    match product {
        0xC52B | 0xC532 | 0xC534 | 0xC537 | 0xC53F => "Unifying",
        0xC539 | 0xC53D | 0xC53A => "Lightspeed",
        0xC545 | 0xC547 | 0xC548 | 0xC54D => "Bolt",
        0xC517 | 0xC51A => "Nano",
        _ => "Receiver",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hid_id_is_split_into_vendor_and_product() {
        assert_eq!(
            parse_hid_id(Some(&"0003:0000046D:0000C547".to_string())),
            Some((0x046D, 0xC547))
        );
        assert_eq!(
            parse_hid_id(Some(&"0003:0000046D:00000B1F".to_string())),
            Some((0x046D, 0x0B1F))
        );
        assert_eq!(parse_hid_id(None), None);
        assert_eq!(parse_hid_id(Some(&"nonsense".to_string())), None);
    }

    #[test]
    fn usb_interface_is_taken_from_hid_phys() {
        assert_eq!(parse_interface("usb-0000:02:00.0-10/input2:1"), Some(2));
        assert_eq!(parse_interface("usb-0000:0b:00.3-2/input3"), Some(3));
        assert_eq!(parse_interface("usb-0000:02:00.0-7/input0"), Some(0));
    }

    #[test]
    fn unique_ids_are_normalised_and_zero_ids_are_none() {
        assert_eq!(normalise_unique("34-81-81-a8").as_deref(), Some("348181A8"));
        assert_eq!(normalise_unique("f9-9a-78-3d").as_deref(), Some("F99A783D"));
        assert_eq!(normalise_unique("0000000000000000"), None);
    }

    #[test]
    fn receivers_are_named_by_their_product_id() {
        assert_eq!(receiver_kind(0xC547), "Bolt");
        assert_eq!(receiver_kind(0xC539), "Lightspeed");
        assert_eq!(receiver_kind(0xC534), "Unifying");
        assert_eq!(receiver_kind(0x1234), "Receiver");
    }
}
