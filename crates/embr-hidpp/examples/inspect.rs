//! Interrogate one hidraw node about the things we cannot guess from a spec: how slow it is
//! to answer, which features it really has, and what its battery features reply with.
//!
//! ```text
//! cargo run --example inspect -p hidpp -- /dev/hidraw0
//! ```
//!
//! It exists because the acceptance probe only reports "no answer within N ms", which cannot
//! tell a sleeping device from an impatient reader — and because the headset on the
//! development machine reads its battery through `ADC MEASUREMENT {1F20}` rather than the
//! `0x10xx` features, with a payload worth looking at before it is decoded.

use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use embr_hidpp::report::{DEVICE_SELF, Feature, FeatureCall};
use embr_hidpp::transport::Channel;

fn main() {
    let path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/hidraw0"));
    let mut channel = match Channel::open(&path) {
        Ok(channel) => channel,
        Err(err) => {
            eprintln!("cannot open {}: {err}", path.display());
            std::process::exit(2);
        }
    };
    println!("opened {}", path.display());

    liveness(&mut channel);
    raw_reports(&mut channel);
    feature_set(&mut channel);
    battery_features(&mut channel);
}

/// The same request sent four ways, to find out which shape the device answers.
///
/// The report id is the interesting variable: HID++ 2.0 has a short (7-byte) and a long
/// (20-byte) request, and a device is free to answer only one of them. The software id is the
/// other: this crate rotated 1..=7 while the reference implementation uses a single fixed id
/// with bit 3 set, and a device that ignores the low ids looks completely silent.
fn raw_reports(channel: &mut Channel) {
    println!(
        "\nraw ROOT.getFeature({:#06X}) — same request, different report sizes and software ids:",
        Feature::AdcMeasurement.id()
    );
    for software_id in [0x01u8, 0x08, 0x0B] {
        let mut call = FeatureCall::new(
            DEVICE_SELF,
            0x00,
            0x00,
            &Feature::AdcMeasurement.id().to_be_bytes(),
        );
        call.software_id = software_id;
        for (label, frame) in [
            ("short", call.to_short().to_vec()),
            ("long ", call.to_long().to_vec()),
        ] {
            let started = Instant::now();
            match channel.call_raw(&frame, Duration::from_millis(1500)) {
                Ok(reply) => println!(
                    "  {label} report, software id {software_id:#04X} -> {} ms  {reply:02X?}",
                    started.elapsed().as_millis()
                ),
                Err(err) => println!("  {label} report, software id {software_id:#04X} -> {err}"),
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }
}

/// ROOT is mandatory in HID++ 2.0, so pinging it measures the device's latency rather than
/// whether it happens to implement one particular feature.
fn liveness(channel: &mut Channel) {
    println!("\nliveness — ROOT {{0000}} at index 0, function 0:");
    for patience in [250u64, 700, 1500, 3000] {
        let started = Instant::now();
        let result = channel.call_feature(
            DEVICE_SELF,
            0x00,
            0x00,
            &Feature::Root.id().to_be_bytes(),
            Duration::from_millis(patience),
        );
        match result {
            Ok(reply) => println!(
                "  waited up to {patience:>4} ms -> answered in {:>5} ms   payload {:02X?}",
                started.elapsed().as_millis(),
                reply.payload
            ),
            Err(err) => println!("  waited up to {patience:>4} ms -> {err}"),
        }
    }
}

/// The device's own feature list: count from function 0, then one call per index.
fn feature_set(channel: &mut Channel) {
    println!("\nfeature set — {{0001}} function 0 (count) and function 1 (id at index):");
    let count = channel
        .call_feature(DEVICE_SELF, 0x01, 0x00, &[], Duration::from_millis(3000))
        .ok()
        .and_then(|reply| {
            reply
                .payload_or_error()
                .ok()
                .and_then(|p| p.first().copied())
        })
        .unwrap_or(0);
    println!("  device reports {count} feature(s)");
    for index in 0..count {
        let Ok(reply) = channel.call_feature(
            DEVICE_SELF,
            0x01,
            0x01,
            &[index],
            Duration::from_millis(3000),
        ) else {
            println!("  {index:>2}: <no answer>");
            continue;
        };
        match reply.payload_or_error() {
            Ok(payload) if payload.len() >= 2 => {
                let id = u16::from_be_bytes([payload[0], payload[1]]);
                let version = payload.get(2).map(|v| v & 0x0F).unwrap_or_default();
                println!(
                    "  {index:>2}: {id:04X}  v{version}  {}",
                    Feature::name_of(id)
                );
            }
            Ok(payload) => println!("  {index:>2}: short payload {payload:02X?}"),
            Err(err) => println!("  {index:>2}: {err}"),
        }
    }
}

/// Every battery feature, called three times a second apart: the headset answers this only
/// while it is in use, so one attempt proves nothing.
fn battery_features(channel: &mut Channel) {
    println!("\nbattery features — index lookup, then function 0 (three attempts, 1 s apart):");
    for feature in [
        Feature::BatteryStatus,
        Feature::BatteryVoltage,
        Feature::UnifiedBattery,
        Feature::AdcMeasurement,
    ] {
        let index = channel
            .call_feature(
                DEVICE_SELF,
                0x00,
                0x00,
                &feature.id().to_be_bytes(),
                Duration::from_millis(3000),
            )
            .ok()
            .and_then(|reply| {
                reply
                    .payload_or_error()
                    .ok()
                    .and_then(|p| p.first().copied())
            })
            .filter(|index| *index != 0);
        let Some(index) = index else {
            println!(
                "  {:<22} {:#06X}  not supported",
                feature.name(),
                feature.id()
            );
            continue;
        };
        println!(
            "  {:<22} {:#06X}  index {index}",
            feature.name(),
            feature.id()
        );
        for attempt in 1..=3 {
            let started = Instant::now();
            let result =
                channel.call_feature(DEVICE_SELF, index, 0x00, &[], Duration::from_millis(3000));
            match result {
                Ok(reply) => match reply.error_code() {
                    Some(code) => println!(
                        "    attempt {attempt}: protocol error {code:#04X} after {} ms",
                        started.elapsed().as_millis()
                    ),
                    None => println!(
                        "    attempt {attempt}: {:02X?} after {} ms",
                        reply.payload,
                        started.elapsed().as_millis()
                    ),
                },
                Err(err) => println!("    attempt {attempt}: {err}"),
            }
            std::thread::sleep(Duration::from_millis(1000));
        }
    }
}
