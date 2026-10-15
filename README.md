# Embr

[![ci](https://github.com/jensend6965/embr/actions/workflows/ci.yml/badge.svg)](https://github.com/jensend6965/embr/actions/workflows/ci.yml)

A Logitech device monitor for Linux. It shows what every attached Logitech mouse, keyboard and
headset is doing — charge, voltage, charge state, connection — in a small native window, a tray
icon and opt-in desktop notifications.

Written from scratch in Rust, talking to HID++ over `/dev/hidraw*` directly: no Python, no
hidapi, no GTK, no D-Bus daemon. One binary, `libc` as its only protocol-layer dependency.

![Embr's window: three devices with battery levels, and the details card for the selected one](docs/screenshot.png)

*A sleeping device says so ("Not answering") instead of showing a stale number.*

## Status

Early, and honest about it: **0.1.0**, no releases and no packaging yet. It reads batteries
well, on real hardware, and the configuration half of the product (DPI, RGB, host switching,
buttons) has not started.

What works today:

* **Battery for every device it can reach** — including headsets, which do not answer the
  battery features mice and keyboards use (see `ADC_MEASUREMENT` below).
* **Nothing invented.** A device that answered but has no reading to give is *Connected*; one
  that did not answer is *Not answering*; a percentage inferred from voltage is marked `~`, and
  a percentage the device reports itself always wins.
* **Tray icon and notifications** (notifications are off until you turn them on), with
  thresholds, hysteresis, once-per-crossing de-duplication, rate limiting and snooze.
* **A 4 kB TOML config** at `~/.config/embr/config.toml`.

## Build and run

```bash
# Ubuntu/Debian: Slint's font/input stack, and libdbus for the tray item (`ksni` 0.2 links the
# C library — `libdbus-1-dev` is what a bare image is missing first)
sudo apt install libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libdbus-1-dev

cargo build --release
./target/release/embr
```

The tests need neither hardware nor a display (they run against captured frames and an injected
clock):

```bash
cargo test --workspace        # 58 tests
cargo clippy --workspace --all-targets
```

### Device access

Reading a battery means opening `/dev/hidraw*`, which is root-only by default. Install the
shipped udev rule to hand those nodes to the logged-in user via `uaccess` — no root program, no
system service:

```bash
sudo install -m 0644 rules.d/70-embr-hidraw.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=hidraw
```

## How it is put together

```
crates/embr-hidpp   protocol: hidraw transport, HID++ framing, receivers, battery decode
                    no display, no event loop, no async; tested against captured frames
crates/embr-core    the brain: config on disk, battery policy, view model, Monitor
                    no UI toolkit, and no HID++ I/O in anything that is tested
crates/embr-app     Slint window + tray + notifications; glue only, no decisions
spikes/             five throwaway experiments behind the decisions, each with a verdict
```

**The app draws, it does not decide.** Thresholds, hysteresis, de-duplication, sorting,
"worst battery first", the wording of a notification: all of it lives in `embr-core`, tested in
microseconds with an injected clock. Neither `embr-core` nor `embr-hidpp` imports a UI toolkit,
and none of the three crates imports the others' concerns.

Speed worth knowing: **3 devices in ~1–2 s**, window painted **22 ms** after launch, **~22 MiB**
release binary, ~95 MB RSS.

## Protocol notes

Learned by capturing frames from real Logitech hardware. These are the traps that cost hours and
are invisible in the vendor's documentation, so they are written down here rather than in a
commit message:

1. **Feature calls to HID++ 2.0 devices go out as long 20-byte reports**, whatever the payload
   size — a 2-byte ROOT lookup included. The report size is a property of the device, not of what
   you are sending. Sent short, a G733 headset answers nothing at any timeout; sent long, it
   answers in 2 ms.
2. **Register reads go out as short 7-byte reports**, even for registers above `0xFF`; these
   receivers do not answer a long-form register read. Register request ids add `0x200` above
   `0x7F`, so register `0x2B5` arrives as `0x83B5`.
3. A device paired to a receiver **answers under its slot index (0x01…), even when the request
   goes to the device's own hidraw node** — a matcher that only accepts `0xFF` discards
   everything and the device looks offline.
4. Replies must be matched on feature **and** function **and** a rotating software id (bit 3
   set, `0x8..=0xF`), with the socket drained first: a wireless round trip can exceed a second,
   and a late reply otherwise matches the *next* request and produces plausible-but-wrong
   numbers.
5. **`DEVICE NAME {0005}` is three functions**: 0 returns the name length, 1 returns a 16-byte
   fragment at the offset you pass, 2 returns the device-type byte. That byte is authoritative —
   guessing "Mouse" from the product string gets short names like `G703 LS` wrong.
6. **Headsets report their battery through `ADC_MEASUREMENT {1F20}`**, not `BATTERY_STATUS
   {1000}`/`BATTERY_VOLTAGE {1001}`/`UNIFIED_BATTERY {1004}`. Its payload is the same
   `millivolts, flags` shape but the flags mean something else (bit 0 = a battery exists, bit 1 =
   charging), and the call errors while the headset is connected but idle.
7. A receiver exposes **three hidraw nodes** and only the HID++ interface answers; the others
   fail with `EPIPE`. Group them by USB port and talk to the interface with the highest index.
8. **An error payload and a data payload can be byte-identical in shape.** Some firmware answers
   an unsupported call with `<0x0F> <code> <0x00>` while a battery reply is
   `<mv_hi> <mv_lo> <flags>` — and `0x0F` is a valid millivolt high byte. Only the documented
   `0x8F` marker is authoritative.

`cargo run --example inspect -p embr-hidpp -- /dev/hidraw0` interrogates a node when a device is
silent: liveness at four timeouts, the whole feature set, and every battery feature three times
over.

## Why Slint, and why a rewrite

Measured before choosing (all five spikes are in `spikes/`, verdicts included):

| Candidate | Cold build | Stripped binary | RSS | To window | Lines | Tray / a11y / i18n |
| --- | --- | --- | --- | --- | --- | --- |
| **Slint 1.18** | 51 s | **21.9 MiB** | 87 MiB | **25 ms** | 462 `.slint` + 57 Rust | built in / built in / built in |
| iced 0.14 | 37.9 s | 15.4 MiB | 73 MiB | 76–209 ms | 506 | no / no / no |
| egui 0.36 | 45.7 s | 19.0 MiB | 75 MiB | 45 ms | 576 (397 UI) | no / no / no |

Slint won on the numbers and on the screenshots: a retained toolkit with real layout, styling
and accessibility, and the least hand-written UI code. The iced and egui prototypes needed
chrome painted by hand and read less finished.

A rewrite rather than a port, because Solaar's UI and its HID++ session live in one Python
process, its toolkit is GTK3 with no GTK4 track, and its hardware layer
(`logitech_receiver`, 21 000 lines, ~800 tests) is a *reference to read*, not a dependency to
import. None of its code is used here.

## Hardware this was developed against

| Device | Connection | What to expect |
| --- | --- | --- |
| G913 TKL keyboard | Bolt receiver `046d:c547`, slot 1 | ~93 %, 4106 mV, discharging |
| G703 LIGHTSPEED mouse | Lightspeed receiver `046d:c539`, slot 1 | ~83 %, 4015 mV, discharging |
| G733 Gaming Headset | USB direct (`046d:0b1f`) | ~61 %, 3866 mV via `ADC_MEASUREMENT` while in use; "Connected, no reading" while idle |

`cargo run --release --example probe -p embr-hidpp` prints the same facts headlessly, so any
report is comparable against `solaar show` on the same machine in the same minute.

## Roadmap

* **Configuration**, in ascending order of risk: DPI (`{2201}`) → RGB effects (`{8071}`) → host
  switching (`{1814}`) → button diversion (needs evdev/uinput rather than HID++ alone).
* **Packaging**: `.desktop` entry, systemd user unit, udev rule installation, releases.
* **A background daemon** so notifications work with the window closed — the crate split is
  already shaped for it.
* **Translation**, through Slint's gettext support.
* **A voltage curve measured from our own devices** (see the licence note below).

## Licence

**MIT OR Apache-2.0**, at your option — see `LICENSE-MIT` and `LICENSE-APACHE`.

Two things to know:

* **Slint** is used under its Royalty-free licence, which requires the `AboutSlint` widget to be
  reachable from an About dialog. Embr's About dialog has it; keep it there.
* **One value in this repository is derived from Solaar** (GPL-2.0-or-later): the
  voltage→percentage anchors in `crates/embr-hidpp/src/battery.rs`, reproduced so that an
  inferred percentage equals what `solaar show` prints for the same millivolts. It is stated in
  the code and here rather than left implicit. Replacing those anchors with a curve measured from
  our own devices removes the question entirely, and is on the roadmap.
