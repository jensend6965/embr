//! Minimal HID++ transport + protocol over raw hidraw fds.
//!
//! Wire format verified against Solaar 1.1.20's own debug trace
//! (evidence/solaar-show-debug.log) and lib/logitech_receiver/base.py:
//!
//!   short report 0x10: [0x10, dev_index, feature_idx, fn|sw_id, params..]  (7 bytes)
//!   long  report 0x11: [0x11, dev_index, feature_idx, fn|sw_id, params..]  (20 bytes)
//!   HID++ 2.0 request id (16 bit) = (feature_idx << 8) | ((function << 4) | sw_id)
//!   HID++ 1.0 register request id = 0x8100 | (register & 0x2FF)
//!       (a "2" in front of a register number marks it a long register, so
//!        register 0x2B5 is sent as request id 0x83B5 — NOT 0x81B5)
//!   reply: data[2..] is the payload; match payload[0..2] on the request bytes
//!   HID++ 2.0 error reply : [0xFF, feature_idx, fn|sw_id, error_code, ..]
//!   HID++ 1.0 error reply : [0x8F, req_id_hi, req_id_lo, error_code, 0x00]
//!   device index: paired devices answer with their receiver slot index (1..6)
//!       even when the request went out on the device's own node.
//!
//! Nothing here is more than framing, a match loop and a retry budget — no
//! hidapi, no HID++ crate.

use std::collections::HashSet;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

pub const REPORT_SHORT: u8 = 0x10;
pub const REPORT_LONG: u8 = 0x11;
pub const DEV_INDEX_SELF: u8 = 0xFF; // "the device this node belongs to"

pub const FEAT_ROOT: u16 = 0x0000;
pub const FEAT_BATTERY_STATUS: u16 = 0x1000;
pub const FEAT_BATTERY_VOLTAGE: u16 = 0x1001;
pub const FEAT_UNIFIED_BATTERY: u16 = 0x1004;
pub const FEAT_ADC_MEASUREMENT: u16 = 0x1F20;

pub const REG_BATTERY_STATUS_10: u16 = 0x07; // HID++ 1.0 battery status register
pub const REG_BATTERY_CHARGE_10: u16 = 0x0D; // HID++ 1.0 battery charge register
pub const REG_RECEIVER_INFO: u16 = 0x2B5; // long register, sub-registered per paired device

/// HID++ 1.0 error codes (logitech_receiver/hidpp10_constants.py: ErrorCode)
pub fn hidpp10_error_name(code: u8) -> &'static str {
    match code {
        0x00 => "SUCCESS",
        0x01 => "INVALID_SUB_ID_COMMAND",
        0x02 => "INVALID_ADDRESS",
        0x03 => "INVALID_VALUE",
        0x04 => "CONNECTION_REQUEST_FAILED",
        0x05 => "TOO_MANY_DEVICES",
        0x06 => "ALREADY_EXISTS",
        0x07 => "BUSY",
        0x08 => "UNKNOWN_DEVICE",
        0x09 => "RESOURCE_ERROR",
        0x0A => "REQUEST_UNAVAILABLE",
        0x0B => "UNSUPPORTED_PARAMETER_VALUE",
        0x0C => "WRONG_PIN_CODE",
        _ => "?",
    }
}

/// HID++ 2.0 error codes (logitech_receiver/hidpp20_constants.py: ErrorCode)
pub fn hidpp20_error_name(code: u8) -> &'static str {
    match code {
        0x00 => "NO_ERROR",
        0x01 => "UNKNOWN",
        0x02 => "INVALID_ARGUMENT",
        0x03 => "OUT_OF_RANGE",
        0x04 => "HW_ERROR",
        0x05 => "NOT_ALLOWED",
        0x06 => "UNSUPPORTED",
        0x07 => "LOGITECH_ERROR",
        0x08 => "BUSY",
        0x09 => "INCOMPLETE",
        0x0A => "UNSUPPORTED_PARAMETER",
        _ => "?",
    }
}

#[derive(Debug)]
pub enum Error {
    Os(io::Error),
    /// HID++ 2.0 feature-call error reply
    Hidpp { feature: u8, function: u8, code: u8 },
    /// HID++ 1.0 error reply (0x8F frame)
    Hidpp10 { request_id: u16, code: u8 },
    /// no matching reply within the retry budget
    Timeout { attempts: u32, ms: u128, last_seen: Option<String> },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Os(e) => write!(f, "io: {e}"),
            Error::Hidpp { feature, function, code } => write!(
                f,
                "HID++ 2.0 error {code:#04x} = {} (feature {feature:#04x} fn {function:#x})",
                hidpp20_error_name(*code)
            ),
            Error::Hidpp10 { request_id, code } => write!(
                f,
                "HID++ 1.0 error {code:#04x} = {} (request {request_id:04X})",
                hidpp10_error_name(*code)
            ),
            Error::Timeout { attempts, ms, last_seen } => {
                write!(f, "no reply after {attempts} attempt(s) / {ms}ms")?;
                if let Some(s) = last_seen {
                    write!(f, "; last unmatched traffic: {s}")?;
                }
                Ok(())
            }
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Os(e)
    }
}

pub struct Hidpp {
    file: File,
    pub node: String,
    /// log every request/reply, not just the first per tag
    pub verbose: bool,
    /// device index the last accepted reply came back with
    pub last_reply_dev: Option<u8>,
    /// rotating software id, so a late reply to a previous request can never be
    /// mistaken for the reply to this one (a wireless round trip can outlive a
    /// request's timeout; see README)
    sw_rot: usize,
    logged: HashSet<String>,
}

/// Software ids keep bit 3 set (Solaar's own is 0x0B) so replies stay
/// distinguishable from notifications, whose sw id is 0.
const SW_IDS: [u8; 8] = [0x0B, 0x0A, 0x09, 0x08, 0x0F, 0x0E, 0x0D, 0x0C];

impl Hidpp {
    pub fn open(node: &str) -> Result<Self, Error> {
        let file = OpenOptions::new().read(true).write(true).open(node)?;
        Ok(Hidpp {
            file,
            node: node.to_string(),
            verbose: false,
            last_reply_dev: None,
            sw_rot: 0,
            logged: HashSet::new(),
        })
    }

    fn next_sw(&mut self) -> u8 {
        let sw = SW_IDS[self.sw_rot % SW_IDS.len()];
        self.sw_rot += 1;
        sw
    }

    fn log(&mut self, tag: &str, text: &str) {
        let first = self.logged.insert(tag.to_string());
        if self.verbose || first {
            eprintln!(
                "[{}] {}{}",
                self.node,
                if first && !self.verbose { "first-attempt: " } else { "" },
                text
            );
        }
    }

    /// Throw away anything already queued (notifications, stale replies).
    fn drain(&mut self) {
        let mut buf = [0u8; 64];
        while self.poll(0) {
            match self.file.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }

    fn poll(&self, timeout_ms: i32) -> bool {
        let mut pfd = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let rv = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        rv > 0 && (pfd.revents & libc::POLLIN) != 0
    }

    fn read_packet(&mut self, timeout_ms: i32) -> io::Result<Option<Vec<u8>>> {
        if !self.poll(timeout_ms) {
            return Ok(None);
        }
        let mut buf = vec![0u8; 64];
        let n = self.file.read(&mut buf)?;
        if n == 0 {
            return Ok(None);
        }
        buf.truncate(n);
        Ok(Some(buf))
    }

    /// Send one HID++ frame and wait for the matching reply, retrying.
    /// `payload` = [feature_idx, fn|sw_id, params..] or [0x81, reg, params..].
    pub fn request_raw(
        &mut self,
        dev_index: u8,
        payload: &[u8],
        long: bool,
        tag: &str,
        attempt_timeout_ms: i32,
        attempts: u32,
    ) -> Result<Vec<u8>, Error> {
        let size = if long { 20 } else { 7 };
        if payload.len() + 2 > size {
            return self.request_raw(dev_index, payload, true, tag, attempt_timeout_ms, attempts);
        }
        let mut frame = vec![0u8; size];
        frame[0] = if long { REPORT_LONG } else { REPORT_SHORT };
        frame[1] = dev_index;
        frame[2..2 + payload.len()].copy_from_slice(payload);

        self.drain();
        self.last_reply_dev = None;
        let mut seen: Vec<String> = Vec::new();
        let start = Instant::now();

        for _attempt in 0..attempts {
            self.log(
                tag,
                &format!("-> {:02X} {:02X} {}", frame[0], frame[1], hex(&frame[2..])),
            );
            self.file.write_all(&frame)?;
            let deadline = Instant::now() + Duration::from_millis(attempt_timeout_ms as u64);
            while Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let ms = (remaining.as_millis().min(i32::MAX as u128) as i32).max(1);
                match self.read_packet(ms) {
                    Ok(None) => break,
                    Ok(Some(data)) => {
                        let d = hex(&data);
                        let ok_len = match data[0] {
                            REPORT_SHORT => data.len() == 7,
                            REPORT_LONG => data.len() == 20,
                            _ => false,
                        };
                        if !ok_len {
                            self.log(tag, &format!("<- {d} (ignored: not a HID++ frame)"));
                            continue;
                        }
                        let reply_dev = data[1];
                        // Solaar accepts the request index or its complement; devices
                        // answering on their own node report their pairing index
                        // (observed: request 0xFF, reply 0x01), so accept those too.
                        let dev_ok = reply_dev == dev_index
                            || reply_dev == dev_index ^ 0xFF
                            || (dev_index == DEV_INDEX_SELF && reply_dev < 0x10);
                        if !dev_ok {
                            self.log(tag, &format!("<- {d} (ignored: dev index {reply_dev:#04x})"));
                            continue;
                        }
                        let p = &data[2..];
                        // HID++ 1.0 error: [0x8F, req_hi, req_lo, code, 0x00]
                        if p.len() >= 4 && p[0] == 0x8F && p[1..3] == payload[..2] {
                            let request_id = u16::from_be_bytes([p[1], p[2]]);
                            self.log(tag, &format!("<- {d} (HID++ 1.0 error)"));
                            return Err(Error::Hidpp10 { request_id, code: p[3] });
                        }
                        // HID++ 2.0 feature-call error: [0xFF, feature, fn|sw, code, ..]
                        if p.len() >= 4 && p[0] == 0xFF && p[1..3] == payload[..2] {
                            self.log(tag, &format!("<- {d} (HID++ 2.0 error)"));
                            return Err(Error::Hidpp { feature: payload[0], function: payload[1] >> 4, code: p[3] });
                        }
                        if p.len() >= 2 && p[..2] == payload[..2] {
                            let note = if reply_dev != dev_index {
                                format!(" (reply dev index {reply_dev:#04x}, requested {dev_index:#04x})")
                            } else {
                                String::new()
                            };
                            self.log(tag, &format!("<- {d}  MATCH{}", note));
                            self.last_reply_dev = Some(reply_dev);
                            return Ok(p[2..].to_vec());
                        }
                        self.log(tag, &format!("<- {d} (ignored: unmatched)"));
                        seen.push(d);
                    }
                    Err(e) => {
                        self.log(tag, &format!("<- read error: {e}"));
                        break;
                    }
                }
            }
        }
        Err(Error::Timeout { attempts, ms: start.elapsed().as_millis(), last_seen: seen.last().cloned() })
    }

    /// HID++ 2.0 feature call: request id = (feature << 8) | ((function << 4) | sw_id)
    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &mut self,
        dev_index: u8,
        feature_idx: u8,
        function: u8,
        params: &[u8],
        long: bool,
        tag: &str,
        attempt_timeout_ms: i32,
        attempts: u32,
    ) -> Result<Vec<u8>, Error> {
        let mut payload = vec![feature_idx, (function << 4) | self.next_sw()];
        payload.extend_from_slice(params);
        self.request_raw(dev_index, &payload, long, tag, attempt_timeout_ms, attempts)
    }

    /// HID++ 1.0 register read: request id = 0x8100 | (register & 0x2FF).
    /// A register numbered 0x2xx is a long register; its id therefore becomes 0x83xx.
    #[allow(clippy::too_many_arguments)]
    pub fn read_register(
        &mut self,
        dev_index: u8,
        register: u16,
        params: &[u8],
        long: bool,
        tag: &str,
        attempt_timeout_ms: i32,
        attempts: u32,
    ) -> Result<Vec<u8>, Error> {
        let request_id = 0x8100u16 | (register & 0x2FF);
        let mut payload = vec![(request_id >> 8) as u8, (request_id & 0xFF) as u8];
        payload.extend_from_slice(params);
        self.request_raw(dev_index, &payload, long, tag, attempt_timeout_ms, attempts)
    }
}

/// ROOT.getFeature(0x0000, function 0) -> (feature_index, feature_type, version)
/// Returns Ok(None) when the device answers with feature index 0 (unsupported).
pub fn get_feature(
    h: &mut Hidpp,
    dev_index: u8,
    feature: u16,
    long: bool,
    tag: &str,
    t_ms: i32,
    attempts: u32,
) -> Result<Option<(u8, u8, u8)>, Error> {
    let reply = h.request(dev_index, FEAT_ROOT as u8, 0x00, &feature.to_be_bytes(), long, tag, t_ms, attempts)?;
    if reply.len() < 3 || reply[0] == 0 {
        return Ok(None);
    }
    Ok(Some((reply[0], reply[1], reply[2])))
}

/// Probe whether a node is the HID++ channel of a receiver: reading the long
/// receiver-info register (0x2B5, sub-register 0x03) only answers there.
/// Observed on this machine: interface :1.2 nodes answer, :1.0/:1.1 do not.
pub fn probe_receiver(h: &mut Hidpp, t_ms: i32, attempts: u32) -> Result<bool, Error> {
    // Short reports first: Solaar sends HID++ 1.0 register reads to a receiver
    // as 7-byte 0x10 reports (its `hidpp_long` is only set for 2.0 devices), and
    // the receivers here only answer that form for registers.
    for long in [false, true] {
        let tag = format!("receiver-probe-{}", if long { "long" } else { "short" });
        match h.read_register(DEV_INDEX_SELF, REG_RECEIVER_INFO, &[0x03], long, &tag, t_ms, attempts) {
            Ok(_) => return Ok(true),
            Err(Error::Hidpp10 { .. }) => continue,   // answered, wrong sub-command: still the HID++ channel
            Err(Error::Os(e)) => return Err(Error::Os(e)),
            Err(_) => continue,
        }
    }
    Ok(false)
}

pub fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

/// ASCII rendering of a byte run, for the device-name register.
pub fn ascii(data: &[u8]) -> String {
    data.iter().map(|b| if (0x20..0x7F).contains(b) { *b as char } else { '.' }).collect()
}
