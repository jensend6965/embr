//! Headless device report — the acceptance check for the protocol layer.
//!
//! Run it against real hardware:
//!
//! ```text
//! cargo run --example probe -p hidpp
//! ```
//!
//! It prints what sysfs shows, what each receiver's pairing table says, and the battery
//! reading for every device — including, honestly, the ones that do not answer.

use hidpp::report::Feature;
use hidpp::sysfs::{self, Role};
use hidpp::transport::{Channel, DEVICE_TIMEOUT, RECEIVER_TIMEOUT};
use hidpp::{discover, receiver};

fn main() {
    let nodes = sysfs::logitech_nodes();
    println!("sysfs: {} Logitech hidraw node(s)", nodes.len());
    for node in &nodes {
        let role = match &node.role {
            Role::Receiver => "receiver".to_string(),
            Role::PairedDevice { receiver_dir } => {
                format!(
                    "paired device (via {})",
                    receiver_dir
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                )
            }
            Role::Direct => "direct".to_string(),
        };
        println!(
            "  {:<14} {:04X}:{:04X}  iface {:?}  driver {:<22} {:<12} {}",
            node.hidraw.display(),
            node.vendor,
            node.product,
            node.usb_interface,
            node.driver,
            role,
            node.name
        );
    }

    let report = discover::discover();
    println!("\ndevices: {}", report.devices.len());
    println!(
        "{:<22} {:<10} {:<26} {:>5} {:>7} {:<13} notes",
        "name", "kind", "connection", "%", "mV", "status"
    );
    for device in &report.devices {
        println!(
            "{:<22} {:<10} {:<26} {:>5} {:>7} {:<13} {}",
            truncate(&device.name, 22),
            device.kind.label(),
            truncate(&device.connection.label(), 26),
            device.percent_text(),
            device
                .battery
                .as_ref()
                .and_then(|b| b.millivolts)
                .map(|mv| mv.to_string())
                .unwrap_or_else(|| "—".to_string()),
            device.status_label(),
            device.error.clone().unwrap_or_default(),
        );
    }

    // Capabilities of the first device that answers: the groundwork for the settings UI.
    for device in &report.devices {
        let Some(path) = device.hidraw.as_ref() else {
            continue;
        };
        let Ok(mut channel) = Channel::open(path) else {
            continue;
        };
        let index = match device.connection {
            hidpp::discover::Connection::Receiver { slot, .. } => slot,
            _ => hidpp::report::DEVICE_SELF,
        };
        let caps = discover::capabilities(&mut channel, index, RECEIVER_TIMEOUT);
        if !caps.is_empty() {
            println!(
                "\n{} advertises {} feature(s) we know:",
                device.name,
                caps.len()
            );
            for (feature, version) in caps {
                println!(
                    "  {:<26} id {:04X} version {version}",
                    feature.name(),
                    feature.id()
                );
            }
            break;
        }
    }

    if !report.warnings.is_empty() {
        println!("\nwarnings:");
        for warning in &report.warnings {
            println!("  {warning}");
        }
    }

    // Raw DEVICE NAME reply: the name/kind layout gets pinned from bytes, not guessed.
    for device in &report.devices {
        let Some(path) = device.hidraw.as_ref() else {
            continue;
        };
        let Ok(mut channel) = Channel::open(path) else {
            continue;
        };
        let index = match device.connection {
            hidpp::discover::Connection::Receiver { slot, .. } => slot,
            _ => hidpp::report::DEVICE_SELF,
        };
        if let Ok(Some(feature_index)) =
            discover::feature_index(&mut channel, index, Feature::DeviceName, RECEIVER_TIMEOUT)
            && let Ok(reply) =
                channel.call_feature(index, feature_index, 0x0, &[], RECEIVER_TIMEOUT)
        {
            println!(
                "\n{}: DEVICE NAME {{0005}} at index {feature_index}",
                device.name
            );
            println!("  raw payload : {:02X?}", reply.payload);
            println!(
                "  as text     : {:?}",
                String::from_utf8_lossy(&reply.payload)
            );
        }
        break;
    }

    // DEVICE NAME function 1: the name plus the device-type byte that decides Mouse vs
    // Keyboard vs Headset without guessing from the product string.
    for device in &report.devices {
        let Some(path) = device.hidraw.as_ref() else {
            continue;
        };
        let Ok(mut channel) = Channel::open(path) else {
            continue;
        };
        let index = match device.connection {
            hidpp::discover::Connection::Receiver { slot, .. } => slot,
            _ => hidpp::report::DEVICE_SELF,
        };
        let Ok(Some(feature_index)) =
            discover::feature_index(&mut channel, index, Feature::DeviceName, RECEIVER_TIMEOUT)
        else {
            continue;
        };
        if let Ok(reply) = channel.call_feature(index, feature_index, 0x1, &[], RECEIVER_TIMEOUT) {
            println!("\n{}: DEVICE NAME {{0005}} function 1", device.name);
            println!("  raw payload : {:02X?}", reply.payload);
            println!(
                "  as text     : {:?}",
                String::from_utf8_lossy(&reply.payload)
            );
        }
        println!(
            "  parsed      : {:?}",
            discover::device_name(&mut channel, index, RECEIVER_TIMEOUT)
        );
        break;
    }

    // Raw pairing records, so the receiver metadata reader can be eyeballed too.
    for node in nodes
        .iter()
        .filter(|n| n.role == Role::Receiver && n.is_hidpp_candidate())
    {
        let Ok(mut channel) = Channel::open(&node.hidraw) else {
            continue;
        };
        let (slots, warnings) = receiver::read_slots(&mut channel, RECEIVER_TIMEOUT);
        let capacity = receiver::max_devices(&mut channel, receiver::METADATA_TIMEOUT);
        let raw_receiver_info = channel
            .read_register(
                0xFF,
                receiver::METADATA_REGISTER,
                &[receiver::RECEIVER_INFORMATION, 0, 0],
                receiver::METADATA_TIMEOUT,
            )
            .map(|payload| format!("{payload:02X?}"))
            .unwrap_or_else(|err| format!("<{err}>"));
        println!(
            "\n{} pairing table (capacity: {capacity:?}, receiver-info payload: {raw_receiver_info}):",
            node.hidraw.display()
        );
        for slot in slots {
            println!(
                "  slot {}: wpid {:04X}  serial {}  name {:?}",
                slot.slot,
                slot.wpid.unwrap_or(0),
                slot.serial.as_deref().unwrap_or("—"),
                slot.name.as_deref().unwrap_or("—")
            );
        }
        for warning in warnings {
            println!("  {warning}");
        }
    }

    let _ = Feature::BatteryVoltage; // documentation anchor for the reader
    let _ = DEVICE_TIMEOUT;
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
