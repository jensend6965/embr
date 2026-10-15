//! Reading a receiver's pairing table (HID++ 1.0 register `0x2B5`).
//!
//! A receiver keeps one metadata record per slot: which product is paired (`WPID`), the
//! device's serial number, and the name the user gave it. The three are read with
//! sub-commands derived from the slot number:
//!
//! ```text
//! 0x20 + slot - 1   pairing information: WPID at payload[3..5] (big endian)
//! 0x30 + slot - 1   serial: four bytes, hex, upper case
//! 0x40 + slot - 1   name: length byte then ASCII
//! ```
//!
//! Bolt-class receivers answer a different family (`0x50`/`0x60`/`0x70`), so both are
//! tried and whichever answers is kept.
//!
//! Replies are *validated by shape*, not trusted: an unsupported sub-command comes back as a
//! short error payload, and an empty slot answers with zeros. Anything that does not look
//! like the record asked for is treated as "nothing there".

use std::time::Duration;

use crate::Result;
use crate::report::MAX_DEVICE_SLOTS;
use crate::transport::{Channel, RECEIVER_TIMEOUT};

/// The receiver metadata register.
pub const METADATA_REGISTER: u16 = 0x2B5;

/// Sub-record holding the receiver's own information, including its device capacity.
pub const RECEIVER_INFORMATION: u8 = 0x03;

/// Which sub-command family a receiver answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// `0x20`/`0x30`/`0x40` — the classic family.
    Classic,
    /// `0x50`/`0x60`/`0x70` — used by Bolt-class receivers.
    Bolt,
}

impl Family {
    fn pairing(self) -> u8 {
        match self {
            Family::Classic => 0x20,
            Family::Bolt => 0x50,
        }
    }
    fn serial(self) -> u8 {
        match self {
            Family::Classic => 0x30,
            Family::Bolt => 0x60,
        }
    }
    fn name(self) -> u8 {
        match self {
            Family::Classic => 0x40,
            Family::Bolt => 0x70,
        }
    }
}

/// One slot of a receiver's pairing table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slot {
    pub slot: u8,
    /// The product id of the paired device (`WPID`, shown in Solaar as e.g. `4086`).
    pub wpid: Option<u16>,
    /// Upper-case hex serial, comparable with sysfs `HID_UNIQ`.
    pub serial: Option<String>,
    /// The name, which is usually what the user sees instead of the product string.
    pub name: Option<String>,
}

impl Slot {
    pub fn is_empty(&self) -> bool {
        self.wpid.is_none() && self.serial.is_none() && self.name.is_none()
    }
}

/// Timeout for receiver metadata reads (register `0x2B5`).
///
/// This is local to the receiver — it answers in tens of milliseconds — while an empty slot
/// simply never answers. A short timeout here keeps a full six-slot sweep under a second
/// instead of seconds of dead waiting. Successful reads on the development machine take
/// ~28 ms.
pub const METADATA_TIMEOUT: Duration = Duration::from_millis(250);

/// Read the receiver's capacity: how many devices it can hold.
///
/// A single-slot Lightspeed receiver reports 1 and the Bolt receiver here reports 2; a
/// Unifying one goes up to 6. Knowing the capacity means a sweep probes the slots that exist
/// instead of all six, which removes several pointless timeouts per sweep.
///
/// The record, captured from both receivers on this machine:
///
/// ```text
/// byte 0      sub-record id (0x03)
/// bytes 1..5  receiver serial number
/// byte 5      firmware/kind nibbles
/// byte 6      device capacity   ← what we want
/// byte 7..    firmware versions, flags
/// ```
pub fn max_devices(channel: &mut Channel, timeout: Duration) -> Option<u8> {
    let payload = channel
        .read_register(
            0xFF,
            METADATA_REGISTER,
            &[RECEIVER_INFORMATION, 0x00, 0x00],
            timeout,
        )
        .ok()?;
    if payload.first().copied()? != RECEIVER_INFORMATION {
        return None;
    }
    let capacity = *payload.get(6)?;
    (1..=MAX_DEVICE_SLOTS)
        .contains(&capacity)
        .then_some(capacity)
}

/// Read the whole pairing table, skipping empty slots.
///
/// The caller's timeout is capped at [`METADATA_TIMEOUT`]: waiting 4 s for a slot that holds
/// no device is pure latency, and there are six of them. Slots beyond the receiver's own
/// capacity are not probed at all.
pub fn read_slots(channel: &mut Channel, timeout: Duration) -> (Vec<Slot>, Vec<String>) {
    let timeout = timeout.min(METADATA_TIMEOUT);
    let capacity = max_devices(channel, timeout).unwrap_or(MAX_DEVICE_SLOTS);
    let mut slots = Vec::new();
    let mut warnings = Vec::new();
    let mut family: Option<Family> = None;
    let mut quiet_slots = 0;
    for slot in 1..=capacity {
        match read_slot(channel, slot, timeout, family) {
            Ok((read, used)) => {
                family = Some(used);
                if !read.is_empty() {
                    slots.push(read);
                }
            }
            // An empty slot simply does not answer; that is not a problem worth reporting.
            Err(crate::Error::Timeout { .. }) | Err(crate::Error::Protocol1 { .. }) => {
                quiet_slots += 1
            }
            Err(err) => warnings.push(format!("slot {slot}: {err}")),
        }
    }
    if slots.is_empty() && quiet_slots == usize::from(capacity) {
        warnings.push(format!(
            "no paired device answered in any of the receiver's {capacity} slot(s)"
        ));
    }
    (slots, warnings)
}

/// Read one slot, optionally forcing the sub-command family discovered earlier.
pub fn read_slot(
    channel: &mut Channel,
    slot: u8,
    timeout: Duration,
    family: Option<Family>,
) -> Result<(Slot, Family)> {
    let families: &[Family] = match family {
        Some(Family::Classic) => &[Family::Classic, Family::Bolt],
        Some(Family::Bolt) => &[Family::Bolt, Family::Classic],
        None => &[Family::Classic, Family::Bolt],
    };
    let mut last_error = None;
    for candidate in families {
        match read_with_family(channel, slot, timeout, *candidate) {
            Ok(read) => return Ok((read, *candidate)),
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error.expect("at least one family is always tried"))
}

fn read_with_family(
    channel: &mut Channel,
    slot: u8,
    timeout: Duration,
    family: Family,
) -> Result<Slot> {
    let mut out = Slot {
        slot,
        ..Slot::default()
    };
    let offset = slot.saturating_sub(1);

    // pairing information: sub-command, then WPID at bytes 3..5
    let pairing = channel.read_register(
        0xFF,
        METADATA_REGISTER,
        &[family.pairing() + offset, 0x00, 0x00],
        timeout,
    )?;
    if pairing
        .first()
        .is_some_and(|b| *b == family.pairing() + offset)
        && pairing.len() >= 5
    {
        let wpid = u16::from_be_bytes([pairing[3], pairing[4]]);
        if wpid != 0 {
            out.wpid = Some(wpid);
        }
    } else {
        return Err(crate::Error::Malformed(format!(
            "no pairing record in slot {slot} (payload {pairing:02X?})"
        )));
    }

    // serial: four raw bytes, shown as hex
    if let Ok(serial) = channel.read_register(
        0xFF,
        METADATA_REGISTER,
        &[family.serial() + offset, 0x00, 0x00],
        timeout,
    ) {
        let looks_right = serial
            .first()
            .is_some_and(|b| *b == family.serial() + offset)
            && serial.len() >= 5
            && serial[1..5].iter().any(|b| *b != 0);
        if looks_right {
            out.serial = Some(serial[1..5].iter().map(|b| format!("{b:02X}")).collect());
        }
    }

    // name: length byte then ASCII
    if let Ok(name) = channel.read_register(
        0xFF,
        METADATA_REGISTER,
        &[family.name() + offset, 0x00, 0x00],
        timeout,
    ) {
        let has_header =
            name.first().is_some_and(|b| *b == family.name() + offset) && name.len() >= 2;
        if has_header {
            let length = usize::from(name[1]);
            if length > 0 && name.len() >= 2 + length {
                let text: String = name[2..2 + length].iter().map(|b| char::from(*b)).collect();
                let trimmed = text
                    .trim_matches(|c: char| c == '\0' || c.is_whitespace())
                    .to_string();
                if !trimmed.is_empty() {
                    out.name = Some(trimmed);
                }
            }
        }
    }

    Ok(out)
}

/// The default timeout for receiver metadata reads.
pub fn default_timeout() -> Duration {
    RECEIVER_TIMEOUT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim payloads captured from the two receivers on the development machine.
    #[test]
    fn the_family_subcommands_are_derived_from_the_slot_number() {
        assert_eq!(Family::Classic.pairing(), 0x20);
        assert_eq!(Family::Classic.pairing() + 1, 0x21);
        assert_eq!(Family::Classic.serial() + 1, 0x31);
        assert_eq!(Family::Classic.name() + 1, 0x41);
        assert_eq!(Family::Bolt.name() + 1, 0x71);
    }

    #[test]
    fn g703_pairing_payload_yields_wpid_4086() {
        // 11 FF 83 B5 | 20 07 08 40 86 0C 01 02 B3 …
        let payload = [0x20, 0x07, 0x08, 0x40, 0x86, 0x0C, 0x01, 0x02, 0xB3];
        let sub = payload[0];
        assert_eq!(sub, Family::Classic.pairing());
        assert_eq!(u16::from_be_bytes([payload[3], payload[4]]), 0x4086);
    }

    #[test]
    fn serial_and_name_payloads_parse() {
        // serial: 11 FF 83 B5 | 30 34 81 81 A8 …   -> 348181A8, matching sysfs HID_UNIQ
        let serial = [0x30u8, 0x34, 0x81, 0x81, 0xA8];
        assert_eq!(
            serial[1..5]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<String>(),
            "348181A8"
        );
        // name: 11 FF 83 B5 | 40 07 47 37 30 33 20 4C 53 …  -> "G703 LS"
        let name = [0x40u8, 0x07, 0x47, 0x37, 0x30, 0x33, 0x20, 0x4C, 0x53];
        let length = usize::from(name[1]);
        let text: String = name[2..2 + length].iter().map(|b| char::from(*b)).collect();
        assert_eq!(text, "G703 LS");
    }
}
