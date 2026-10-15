//! Spike 001 — read Logitech battery state directly over HID++ via raw hidraw.
//!
//! Two access routes are attempted for every peripheral:
//!   (a) receiver route: the receiver's HID++ control node + the paired device
//!       index (1..n) discovered from the receiver's pairing register 0x2B5
//!   (b) device route:   the device's own hidraw node, using its receiver slot
//!       index (observed on this hardware; 0xFF is tried as a fallback)
//!
//! Everything is std + libc::poll. No hidapi, no udev, no HID++ crate.

mod battery;
mod hidpp;
mod sysfs;

use battery::{
    decode_adc_measurement, decode_battery_status, decode_battery_voltage, decode_hidpp10_register, decode_unified_battery,
    BatteryInfo,
};
use hidpp::{
    ascii, get_feature, hex, probe_receiver, Error, Hidpp, DEV_INDEX_SELF, FEAT_ADC_MEASUREMENT, FEAT_BATTERY_STATUS,
    FEAT_BATTERY_VOLTAGE, FEAT_UNIFIED_BATTERY, REG_BATTERY_CHARGE_10, REG_BATTERY_STATUS_10, REG_RECEIVER_INFO,
};
use std::time::Instant;
use sysfs::HidrawDevice;

#[derive(Clone)]
struct Cfg {
    verbose: bool,
    /// timeout for feature calls to a device (Solaar: _DEVICE_REQUEST_TIMEOUT = 4s,
    /// a wireless round trip can be slow)
    t_ms: i32,
    /// timeout for calls to the receiver itself (Solaar: _RECEIVER_REQUEST_TIMEOUT = 0.9s)
    t_rx_ms: i32,
    attempts: u32,
    long: bool,
    max_index: u8,
    skip_receiver: bool,
    skip_device: bool,
    skip_extra: bool,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg {
            verbose: false,
            t_ms: 4000,
            t_rx_ms: 900,
            attempts: 2,
            long: true,
            max_index: 6,
            skip_receiver: false,
            skip_device: false,
            skip_extra: false,
        }
    }
}

struct Outcome {
    info: Option<BatteryInfo>,
    notes: Vec<String>,
    error: Option<String>,
    ms: u128,
}

struct Row {
    node: String,
    route: &'static str,
    dev_index: u8,
    name: String,
    out: Outcome,
}

/// One paired-device slot of a receiver, from register 0x2B5.
struct PairSlot {
    index: u8,
    wpid: u16,
    name: Option<String>,
    serial: Option<String>,
    present: bool,
}

fn main() {
    let mut cfg = Cfg::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--raw" => cfg.verbose = true,
            "--short" => cfg.long = false,
            "--no-receiver" => cfg.skip_receiver = true,
            "--no-device" => cfg.skip_device = true,
            "--no-extra" => cfg.skip_extra = true,
            "--timeout" => cfg.t_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(cfg.t_ms),
            "--timeout-rx" => cfg.t_rx_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(cfg.t_rx_ms),
            "--attempts" => cfg.attempts = args.next().and_then(|v| v.parse().ok()).unwrap_or(cfg.attempts),
            "--max-index" => cfg.max_index = args.next().and_then(|v| v.parse().ok()).unwrap_or(cfg.max_index),
            "-h" | "--help" => {
                println!(
                    "battery [--raw] [--short] [--no-receiver] [--no-device] [--no-extra] [--timeout MS] [--timeout-rx MS] [--attempts N] [--max-index N]"
                );
                return;
            }
            other => eprintln!("ignoring unknown argument {other:?}"),
        }
    }

    println!("spike 001 — Rust HID++ battery reader (raw hidraw; deps: libc only)");
    println!(
        "framing: {} reports ({} bytes), device timeout {}ms, receiver timeout {}ms, attempts {}, receiver index sweep 1..={}",
        if cfg.long { "long 0x11" } else { "short 0x10" },
        if cfg.long { 20 } else { 7 },
        cfg.t_ms,
        cfg.t_rx_ms,
        cfg.attempts,
        cfg.max_index
    );
    println!("raw request/reply lines go to stderr, prefixed with the node path\n");

    let devices = sysfs::enumerate();

    println!("--- hidraw nodes from /sys/class/hidraw/*/device/uevent ---");
    println!(
        "{:<14} {:<10} {:<6} {:<9} {:<38} {:<14} {}",
        "node", "vid:pid", "iface", "kind", "name(HID_NAME)", "uniq(HID_UNIQ)", "sysfs parent"
    );
    for d in &devices {
        println!(
            "{:<14} {:04x}:{:04x}  {:<6} {:<9} {:<38} {:<14} {}",
            d.node,
            d.vid,
            d.pid,
            d.interface.map(|i| i.to_string()).unwrap_or_else(|| "-".into()),
            if d.receiver_like { "receiver" } else { "device" },
            truncate(&d.name, 38),
            d.uniq.clone().unwrap_or_else(|| "-".into()),
            d.via_receiver.clone().unwrap_or_else(|| "-".into())
        );
    }
    println!();

    // ---- receiver control-node detection ------------------------------------
    let mut receivers: Vec<HidrawDevice> = Vec::new();
    println!("--- receiver control-node detection (HID++ reg 0x2B5 read at device index 0xFF) ---");
    if cfg.skip_receiver {
        println!("(skipped: --no-receiver)");
    } else {
        for d in devices.iter().filter(|d| d.control_interface_candidate()) {
            match Hidpp::open(&d.node) {
                Ok(mut h) => {
                    h.verbose = cfg.verbose;
                    match probe_receiver(&mut h, cfg.t_rx_ms, cfg.attempts) {
                        Ok(true) => {
                            println!("{:<14} iface {:<3} -> REPLIED — HID++ receiver channel", d.node, iface(d));
                            receivers.push(d.clone());
                        }
                        Ok(false) => println!("{:<14} iface {:<3} -> no reply — not the HID++ channel", d.node, iface(d)),
                        Err(e) => println!("{:<14} iface {:<3} -> probe error: {e}", d.node, iface(d)),
                    }
                }
                Err(e) => println!("{:<14} cannot open: {e}", d.node),
            }
        }
        if receivers.is_empty() {
            println!("(no receiver control node answered)");
        }
    }
    println!();

    // ---- route (a): receiver node + paired device index ---------------------
    let mut rows: Vec<Row> = Vec::new();
    let mut slots_by_receiver: Vec<(String, Vec<PairSlot>)> = Vec::new();
    if !cfg.skip_receiver {
        println!("--- route (a): receiver node, per-index battery query ---");
        for r in &receivers {
            let mut h = match Hidpp::open(&r.node) {
                Ok(h) => h,
                Err(e) => {
                    println!("{}: cannot open: {e}", r.node);
                    continue;
                }
            };
            h.verbose = cfg.verbose;

            let slots = read_pairing_slots(&mut h, &cfg, &r.node);
            println!();
            // If the receiver's pairing register answered for at least one slot,
            // trust it: do not spend battery queries on empty slots.
            let any_present = slots.iter().any(|s| s.present);
            let empty: Vec<String> = slots.iter().filter(|s| !s.present).map(|s| s.index.to_string()).collect();
            if any_present && !empty.is_empty() {
                println!(
                    "{:<14} empty paired-device slots (register 0x2B5 all zero): {}",
                    r.node,
                    empty.join(",")
                );
            }
            for slot in &slots {
                if !slot.present && any_present {
                    continue;
                }
                let t0 = Instant::now();
                let mut out = query_battery(&mut h, slot.index, &cfg, &format!("rx-dev{}", slot.index));
                out.ms = t0.elapsed().as_millis();
                if !slot.present && out.info.is_none() {
                    out.notes.insert(0, "slot empty / not paired per register 0x2B5".to_string());
                }
                if slot.present || out.info.is_some() {
                    if let Some(s) = &slot.serial {
                        out.notes.push(format!("serial {s}"));
                    }
                    rows.push(Row {
                        node: r.node.clone(),
                        route: "receiver",
                        dev_index: slot.index,
                        name: slot.name.clone().unwrap_or_else(|| match slot.wpid {
                            0 => format!("(index {}, unnamed)", slot.index),
                            w => format!("(index {}, wpid {w:04X})", slot.index),
                        }),
                        out,
                    });
                }
            }
            slots_by_receiver.push((r.node.clone(), slots));
            drop(h);
            println!();
        }
    }

    // ---- route (b): the device's own node -----------------------------------
    if !cfg.skip_device {
        println!("--- route (b): device's own hidraw node ---");
        for d in devices.iter().filter(|d| !d.control_interface_candidate()) {
            // If the receiver told us this device's slot index, use it: the
            // firmware answers on its own node under its pairing index.
            let slot_index = slots_by_receiver
                .iter()
                .find(|(node, _)| d.via_receiver.as_deref() == Some(node.as_str()))
                .and_then(|(_, slots)| slots.iter().find(|s| s.present && (s.wpid == d.pid || s.name.as_deref() == Some(&d.name))))
                .map(|s| s.index);
            let mut tried: Vec<(u8, Outcome)> = Vec::new();
            for index in slot_index.into_iter().chain(std::iter::once(DEV_INDEX_SELF)) {
                let t0 = Instant::now();
                let out = match Hidpp::open(&d.node) {
                    Ok(mut h) => {
                        h.verbose = cfg.verbose;
                        let mut o = query_battery(&mut h, index, &cfg, &format!("dev-idx{index:02x}"));
                        o.ms = t0.elapsed().as_millis();
                        o
                    }
                    Err(e) => Outcome {
                        info: None,
                        notes: vec![],
                        error: Some(format!("open failed: {e}")),
                        ms: t0.elapsed().as_millis(),
                    },
                };
                let ok = out.info.is_some();
                tried.push((index, out));
                if ok {
                    break;
                }
            }
            let (index, mut out) = match tried.len() {
                0 => (DEV_INDEX_SELF, Outcome { info: None, notes: vec![], error: Some("not attempted".into()), ms: 0 }),
                1 => tried.pop().unwrap(),
                _ => {
                    let first = tried.first().map(|(i, _)| *i).unwrap_or(DEV_INDEX_SELF);
                    let (_, mut last) = tried.pop().unwrap();
                    let _ = first;
                    last.notes.push("tried the receiver slot index first, then 0xFF".into());
                    (DEV_INDEX_SELF, last)
                }
            };
            if let Some(vr) = &d.via_receiver {
                out.notes.push(format!("sysfs parent node {vr}"));
            }
            if let Some(si) = slot_index {
                out.notes.push(format!("receiver slot {si} from register 0x2B5"));
            }
            rows.push(Row { node: d.node.clone(), route: "device", dev_index: index, name: d.name.clone(), out });
        }
        println!();
    }

    // ---- the table ----------------------------------------------------------
    println!("--- BATTERY TABLE ---");
    let header = format!(
        "{:<15} {:<9} {:<5} {:<30} {:>4} {:>6} {:<15} {:>7}  {}",
        "node", "route", "dev", "name", "%", "mV", "status", "ms", "source / notes"
    );
    println!("{header}");
    println!("{}", "-".repeat(header.len() + 60));
    for r in &rows {
        let (pct, mv, st, src) = match &r.out.info {
            Some(i) => (
                i.percent.map(|v| v.to_string()).unwrap_or_else(|| "?".into()),
                i.millivolts.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
                i.status.map(|s| s.as_str().to_string()).unwrap_or_else(|| "?".into()),
                i.source.clone(),
            ),
            None => ("-".into(), "-".into(), "-".into(), r.out.error.clone().unwrap_or_else(|| "no data".into())),
        };
        let mut note = src;
        for n in &r.out.notes {
            note.push_str(" | ");
            note.push_str(n);
        }
        if let Some(i) = &r.out.info {
            note.push_str(&format!(" | raw[{}]", i.flags_hex));
            if let Some(n) = i.next_level {
                note.push_str(&format!(" next-level {n}"));
            }
            if let Some(approx) = i.level_approx {
                note.push_str(if approx { " level-flag=approximate" } else { " level-flag=exact" });
            }
        }
        println!(
            "{:<15} {:<9} {:#04x}  {:<30} {:>4} {:>6} {:<15} {:>7}  {}",
            r.node,
            r.route,
            r.dev_index,
            truncate(&r.name, 30),
            pct,
            mv,
            st,
            r.out.ms,
            note
        );
    }
    println!();
    println!("legend: dev 0x01..0x06 = receiver slot index, 0xff = the device's own node addressed as itself.");
    println!(
        "status (HID++ 2.0): 0 DISCHARGING 1 RECHARGING 2 ALMOST_FULL 3 FULL 4 SLOW_RECHARGE 5 INVALID_BATTERY 6 THERMAL_ERROR"
    );
}

fn iface(d: &HidrawDevice) -> String {
    d.interface.map(|i| i.to_string()).unwrap_or_else(|| "?".into())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

/// Read the receiver's per-slot info out of long register 0x2B5:
///   sub 0x20+n-1 pairing info  -> fields[3..5] = WPID  (Unifying/Lightspeed order)
///   sub 0x30+n-1 serial        -> fields[1..5] = serial
///   sub 0x40+n-1 friendly name -> fields[1] = length, fields[2..] = name
/// Bolt receivers use sub 0x50+n / 0x60+n instead, so both styles are attempted
/// and whichever answers first is kept.
fn read_pairing_slots(h: &mut Hidpp, cfg: &Cfg, node: &str) -> Vec<PairSlot> {
    let mut styles: Vec<(&'static str, bool)> = vec![
        ("unifying/lightspeed 0x2N short", false),
        ("bolt 0x5N short", false),
        ("unifying/lightspeed 0x2N long", true),
    ];
    let mut slots: Vec<PairSlot> = Vec::new();
    for index in 1..=cfg.max_index {
        let mut hit: Option<(&'static str, bool, Vec<u8>)> = None;
        for (style, long) in styles.clone() {
            let sub = if style.starts_with("bolt") { 0x50 + index } else { 0x20 + index - 1 };
            let tag = format!("pairing-{}-{index}", style.split_whitespace().next().unwrap());
            match h.read_register(DEV_INDEX_SELF, REG_RECEIVER_INFO, &[sub], long, &tag, cfg.t_rx_ms, cfg.attempts) {
                Ok(reply) => {
                    hit = Some((style, long, reply));
                    break;
                }
                Err(Error::Hidpp10 { .. }) => continue,
                Err(_) => continue,
            }
        }
        match hit {
            Some((style, long, fields)) => {
                styles = vec![(style, long)]; // stick with the style that answered
                let present = fields.iter().any(|b| *b != 0);
                let wpid = if style.starts_with("bolt") {
                    let hi = fields.get(3).copied().unwrap_or(0) as u16;
                    let lo = fields.get(2).copied().unwrap_or(0) as u16;
                    hi << 8 | lo
                } else if fields.len() >= 5 {
                    u16::from_be_bytes([fields[3], fields[4]])
                } else {
                    0
                };
                let mut name = None;
                let mut serial = None;
                if present {
                    let (nsub, ssub) = if style.starts_with("bolt") {
                        (0x60 + index, 0x50 + index)
                    } else {
                        (0x40 + index - 1, 0x30 + index - 1)
                    };
                    if let Ok(nf) = h.read_register(
                        DEV_INDEX_SELF,
                        REG_RECEIVER_INFO,
                        &[nsub],
                        long,
                        &format!("name-{index}"),
                        cfg.t_rx_ms,
                        cfg.attempts,
                    ) {
                        let len = nf.get(1).copied().unwrap_or(0) as usize;
                        let text = if len > 0 && nf.len() > 2 + len {
                            ascii(&nf[2..2 + len]).trim().to_string()
                        } else {
                            ascii(&nf[1..]).trim_matches('.').trim().to_string()
                        };
                        name = Some(if text.is_empty() { format!("(index {index}, wpid {wpid:04X})") } else { text });
                    }
                    if let Ok(sf) = h.read_register(
                        DEV_INDEX_SELF,
                        REG_RECEIVER_INFO,
                        &[ssub],
                        long,
                        &format!("serial-{index}"),
                        cfg.t_rx_ms,
                        cfg.attempts,
                    ) {
                        if sf.len() >= 5 {
                            serial = Some(hex(&sf[1..5]).replace(' ', "").to_ascii_uppercase());
                        }
                    }
                }
                println!(
                    "{node:<14} index {index}: style={style} fields=[{}] -> wpid {wpid:04X}{}{}",
                    hex(&fields),
                    name.as_ref().map(|n| format!(" name {n:?}")).unwrap_or_default(),
                    serial.as_ref().map(|s| format!(" serial {s}")).unwrap_or_default()
                );
                slots.push(PairSlot { index, wpid, name, serial, present });
            }
            None => {
                println!("{node:<14} index {index}: no pairing register reply");
                slots.push(PairSlot { index, wpid: 0, name: None, serial: None, present: false });
            }
        }
    }
    slots
}

/// The HID++ 2.0 battery flow: ROOT.getFeature(feature) -> function 0 (or 0x10
/// for UNIFIED_BATTERY). Order and payloads follow Solaar's battery_functions.
/// Falls back to the HID++ 1.0 registers 0x07 / 0x0D for 1.0-era devices.
fn query_battery(h: &mut Hidpp, dev_index: u8, cfg: &Cfg, tag: &str) -> Outcome {
    let t0 = Instant::now();
    let mut notes: Vec<String> = Vec::new();

    let mut calls: Vec<(u16, &'static str, u8)> = vec![
        (FEAT_BATTERY_STATUS, "BATTERY_STATUS {1000}", 0x00),
        (FEAT_BATTERY_VOLTAGE, "BATTERY_VOLTAGE {1001}", 0x00),
    ];
    if !cfg.skip_extra {
        calls.push((FEAT_UNIFIED_BATTERY, "UNIFIED_BATTERY {1004}", 0x10));
        calls.push((FEAT_ADC_MEASUREMENT, "ADC_MEASUREMENT {1F20}", 0x00));
    }

    for (feature, label, function) in calls {
        let ftag = format!("{tag}-root{feature:04x}");
        match get_feature(h, dev_index, feature, cfg.long, &ftag, cfg.t_ms, cfg.attempts) {
            Ok(Some((idx, ftype, ver))) => {
                notes.push(format!("{label} idx {idx} type {ftype:#04x} ver {ver}"));
                let btag = format!("{tag}-f{idx:02x}fn{function:x}");
                match h.request(dev_index, idx, function, &[], cfg.long, &btag, cfg.t_ms, cfg.attempts) {
                    Ok(reply) => {
                        let src = format!("{label} idx {idx} fn{function:x}");
                        let info = match feature {
                            FEAT_BATTERY_STATUS => decode_battery_status(&reply, &src),
                            FEAT_BATTERY_VOLTAGE => decode_battery_voltage(&reply, &src),
                            FEAT_UNIFIED_BATTERY => decode_unified_battery(&reply, &src),
                            _ => decode_adc_measurement(&reply, &src),
                        };
                        if let Some(dev) = h.last_reply_dev {
                            if dev != dev_index {
                                notes.push(format!("reply came from device index {dev:#04x}"));
                            }
                        }
                        match info {
                            Some(info) => return Outcome { info: Some(info), notes, error: None, ms: t0.elapsed().as_millis() },
                            None => notes.push(format!("{label} fn{function:x}: reply too short [{}]", hex(&reply))),
                        }
                    }
                    Err(e) => notes.push(format!("{label} fn{function:x} failed: {e}")),
                }
            }
            Ok(None) => notes.push(format!("{label}: not supported")),
            Err(e) => notes.push(format!("{label}: getFeature failed: {e}")),
        }
    }

    // HID++ 1.0 register fallback (only meaningful for pre-2.0 devices).
    // Register reads to the receiver itself use the receiver timeout (0.9s).
    let t_reg = if dev_index == DEV_INDEX_SELF { cfg.t_rx_ms } else { cfg.t_ms };
    for reg in [REG_BATTERY_STATUS_10, REG_BATTERY_CHARGE_10] {
        for long in [false, true] {
            let rtag = format!("{tag}-reg{reg:02x}");
            match h.read_register(dev_index, reg, &[], long, &rtag, t_reg, cfg.attempts) {
                Ok(reply) => {
                    if let Some(info) = decode_hidpp10_register(&reply, reg as u8, &format!("HID++1.0 register {reg:#04x}")) {
                        notes.push(format!("register {reg:#04x} reply [{}]", hex(&reply)));
                        return Outcome { info: Some(info), notes, error: None, ms: t0.elapsed().as_millis() };
                    }
                }
                Err(e) => notes.push(format!("register {reg:#04x}: {e}")),
            }
        }
    }

    let error = if notes.iter().any(|n| n.contains("getFeature failed")) {
        format!("unreachable / no reply on any feature call")
    } else {
        format!("no battery feature supported")
    };
    Outcome { info: None, notes, error: Some(error), ms: t0.elapsed().as_millis() }
}
