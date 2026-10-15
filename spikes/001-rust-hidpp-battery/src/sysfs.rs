//! Enumerate Logitech HID++ hidraw nodes straight out of sysfs.
//!
//! No libudev: everything we need is in /sys/class/hidraw/<node>/device/uevent
//! (HID_ID carries bus:vid:pid, HID_NAME the product string) plus the sysfs
//! path, which tells us which USB *interface* a node belongs to and whether a
//! device node was spawned underneath a receiver node by hid-logitech-dj.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct HidrawDevice {
    /// e.g. /dev/hidraw6
    pub node: String,
    /// /sys/devices/.../0003:046D:C539.0007
    pub realpath: PathBuf,
    pub vid: u16,
    pub pid: u16,
    /// HID_NAME from sysfs (product string of the USB interface)
    pub name: String,
    /// HID_UNIQ (a device node carries the device serial, e.g. 34-81-81-a8)
    pub uniq: Option<String>,
    /// USB interface number of this node's own interface (:1.N), if derivable
    pub interface: Option<u8>,
    /// HID_NAME says "Receiver" -> candidate receiver control node
    pub receiver_like: bool,
    /// USB product id looks like a known Logitech receiver dongle
    pub receiver_pid: bool,
    /// For a device node: the receiver control node whose sysfs dir contains us
    pub via_receiver: Option<String>,
}

impl HidrawDevice {
    /// Is this the HID++ control interface of a receiver? Logitech receivers
    /// expose the HID++ channel on USB interface 2 (the "1-7:1.2" node); the
    /// other interfaces are the emulated mouse/keyboard endpoints.
    pub fn control_interface_candidate(&self) -> bool {
        self.receiver_like || self.receiver_pid
    }
}

/// Known Logitech receiver/dongle USB product ids (USB_DEVICE_IDs of the
/// receiver, not the paired peripherals).
fn is_receiver_pid(pid: u16) -> bool {
    matches!(
        pid,
        0xC52B // Unifying
            | 0xC532 | 0xC534 | 0xC537 | 0xC539 // Lightspeed (c539: G703/G915 receivers)
            | 0xC53F | 0xC547 | 0xC548 // Bolt
            | 0xC52F | 0xC521 // Nano
            | 0xC51A | 0xC51B
    )
}

fn parse_uevent(text: &str) -> (Option<(u16, u16)>, Option<String>, Option<String>) {
    let mut id = None;
    let mut name = None;
    let mut uniq = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("HID_ID=") {
            // 0003:0000046D:0000C547 -> bus:vendor:product
            let parts: Vec<&str> = v.split(':').collect();
            if parts.len() == 3 {
                if let (Ok(vid), Ok(pid)) =
                    (u16::from_str_radix(parts[1], 16), u16::from_str_radix(parts[2], 16))
                {
                    id = Some((vid, pid));
                }
            }
        } else if let Some(v) = line.strip_prefix("HID_NAME=") {
            name = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("HID_UNIQ=") {
            let v = v.trim();
            if !v.is_empty() && v != "0000000000000000" {
                uniq = Some(v.to_string());
            }
        }
    }
    (id, name, uniq)
}

/// Find the USB interface number (the N in ":1.N") of a sysfs device path.
fn usb_interface(path: &Path) -> Option<u8> {
    for comp in path.components().rev() {
        let s = comp.as_os_str().to_string_lossy();
        if let Some(colon) = s.rfind(':') {
            let rest = &s[colon + 1..];
            if let Some(dot) = rest.find('.') {
                let (cfg, intf) = (&rest[..dot], &rest[dot + 1..]);
                if cfg.parse::<u8>().is_ok() && intf.parse::<u8>().is_ok() {
                    return intf.parse::<u8>().ok();
                }
            }
        }
    }
    None
}

pub fn enumerate() -> Vec<HidrawDevice> {
    let mut out: Vec<HidrawDevice> = Vec::new();
    let entries = match fs::read_dir("/sys/class/hidraw") {
        Ok(e) => e,
        Err(_) => return out,
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("hidraw"))
        .collect();
    names.sort();

    for n in names {
        let uevent = format!("/sys/class/hidraw/{n}/device/uevent");
        let text = match fs::read_to_string(&uevent) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let (id, name, uniq) = parse_uevent(&text);
        let (vid, pid) = id.unwrap_or((0, 0));
        let realpath = fs::canonicalize(format!("/sys/class/hidraw/{n}/device"))
            .unwrap_or_else(|_| PathBuf::from(&uevent));
        let name = name.unwrap_or_default();
        let receiver_like = name.to_ascii_lowercase().contains("receiver");
        out.push(HidrawDevice {
            node: format!("/dev/{n}"),
            interface: usb_interface(&realpath),
            realpath,
            vid,
            pid,
            receiver_like,
            receiver_pid: is_receiver_pid(pid),
            name,
            uniq,
            via_receiver: None,
        });
    }

    // A device node lives *under* the receiver node's sysfs dir (hid-logitech-dj
    // creates a HID node per paired device). Use that to link device -> receiver.
    let parents: Vec<(String, PathBuf)> =
        out.iter().map(|d| (d.node.clone(), d.realpath.clone())).collect();
    for dev in out.iter_mut() {
        if dev.receiver_like {
            continue;
        }
        if let Some(parent) = dev.realpath.parent() {
            for (node, rp) in &parents {
                if rp.as_path() == parent && rp != &dev.realpath {
                    dev.via_receiver = Some(node.clone());
                }
            }
        }
    }
    out
}
