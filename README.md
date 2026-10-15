# Modern Logitech device manager — new Rust project (working name)

This directory is the **new app**: a from-scratch, modern, lightweight Logitech device
manager for Linux — the successor to the GTK3/Python app in `~/Projects/solaar-ui`.

`~/Projects/solaar-ui` stays what it is: a fork of upstream Solaar (GPL-2.0) with our
tooling, tests and the protocol reference we read from. Nothing here modifies it.

## Why a new app instead of a port

Decision taken 2026-10-15 after measuring the alternatives — see
`~/Projects/solaar-ui/PROPOSAL.md` for the full reasoning. Short version:

* Solaar's UI and its HID++ hardware session live in **one Python process**, so battery
  monitoring, notifications and the tray only exist while the app runs.
* Solaar's UI is GTK3; upstream has no GTK4 port and no modern-design track.
* `logitech_receiver` (21 412 LOC, ~800 tests) is the crown jewel and stays as our
  **reference implementation** to read, not a dependency to import.

## Stack — decided 2026-10-15

| Decision | State |
| --- | --- |
| Language | **Rust** — one language across protocol, GUI, tray and CLI; single binary; no interpreter |
| Protocol | **our own**, written from the HID++ documentation and from byte-level measurements: no Solaar code, no hidapi, no udev crate — `libc` is the only dependency |
| GUI | **Slint 1.18** — chosen on the spike numbers and on the prototypes: 25 ms to window, 21.9 MiB binary, tray/i18n/accessibility built in, least UI code |
| Tray + notifications | `ksni` (StatusNotifierItem) + `notify-rust`; both verified on this desktop |
| Licence | MIT OR Apache-2.0 for our code, on the **Slint Royalty-free licence** for the toolkit — which requires the `AboutSlint` widget in an About dialog, implemented in the app. **Exception to check:** `VOLTAGE_CURVE` in `crates/hidpp/src/battery.rs` is reproduced from Solaar (GPL-2.0-or-later) so that percentages match `solaar show` — the provenance is stated in the code, and replacing those anchors with a curve measured from our own devices would settle it |

## Architecture

```
crates/hidpp   protocol: hidraw transport, HID++ framing, receivers, battery decode
               no display, no event loop, no async; unit-tested against captured frames
crates/core    the brain: config on disk, battery policy, view model, Monitor
               no UI toolkit, no HID++ I/O in the logic that is tested
crates/app     Slint window + tray + notifications; glue only, no decisions
spikes/        throwaway experiments behind the decisions (each with a Verdict)
```

The rule that keeps this maintainable: **the app formats and draws, it does not decide.**
Thresholds, hysteresis, de-duplication, sorting, "worst battery first", the wording of a
notification — all of it lives in `core` and is tested in microseconds with an injected
clock. Nothing in `core` or `hidpp` imports a UI toolkit.

## Status (2026-10-15)

* `cargo test --workspace`: **56 tests, 0 failures**; `cargo clippy --workspace
  --all-targets`: clean.
* The app runs: `./target/release/periphery`. Window painted **22 ms** after launch, first
  device data **1.5–2.6 s** later (a sweep costs one short timeouts round per receiver plus
  per-device queries), **~95 MB** RSS, 26 threads, **21.9 MiB** stripped binary. Screenshots
  in `crates/app/evidence/`.
* Sweep latency was 23.5 s when the app was first wired up. Three fixes took it to under two
  seconds, and each one is a protocol fact rather than a micro-optimisation:
  1. receiver metadata reads get a 250 ms timeout (the receiver answers in ~28 ms; an empty
     slot never answers, and there were six of them per receiver);
  2. a sweep probes only the slots the receiver says it has — capacity comes from the
     receiver-information record (`0x2B5` sub `0x03`, byte 6: 1 for the Lightspeed receiver,
     2 for the Bolt, matching `solaar show`);
  3. a directly attached device gets a 700 ms ping before any full-length query, and a device
     that times out once is not asked again in the same sweep.
* Hardware-verified by the headless probe (`cargo run --example probe -p hidpp`):

| Device | Kind | Connection | Reading |
| --- | --- | --- | --- |
| G913 TKL | Keyboard | Bolt (C547, slot 1) | ~95 %, 4117 mV, Discharging |
| G703 LIGHTSPEED… | Mouse | Lightspeed (C539, slot 1) | ~89 %, 4040 mV, Discharging |
| G733 Gaming Headset | Headset | USB | Unavailable — reported, never invented |

* The probe also lists each device's HID++ feature set, which is the groundwork for the
  configuration half of the product: the G703 already advertises **ADJUSTABLE DPI {2201}**,
  **RGB EFFECTS {8071}** and **ONBOARD PROFILES {8100}**, the keyboard adds **SPECIAL KEYS
  {1B04}** and **CHANGE HOST {1814}**.

## Protocol facts this project owns (learned the hard way, on real hardware)

1. Register request ids add `0x200` above `0x7F`: register `0x2B5` goes out as `0x83B5`.
   `0x81B5` earns a HID++ 1.0 error and nothing else.
2. Register reads go out as **short** 7-byte reports even for registers above `0xFF`; these
   receivers do not answer a long-form register read.
3. **Feature calls go out as long 20-byte reports, always** — the size is a property of the
   device (HID++ 2.0 and up), not of the payload. Sending a 2-byte ROOT lookup short cost us
   the whole G733 headset: it answers in 2 ms as a long report and never answers short, at any
   timeout or software id.
4. A device paired to a receiver answers under **its slot index (0x01…) even when the
   request goes to the device's own hidraw node**, so a matcher that only accepts `0xFF`
   discards every reply.
5. Replies must be matched on feature **and** function **and** a rotating software id (with
   bit 3 set, `0x8..=0xF`, so a reply cannot be mistaken for a notification), with a drained
   socket: the receiver's wireless round trip can exceed a second, and a late reply otherwise
   matches the *next* request and produces plausible-but-wrong numbers.
6. `DEVICE NAME {0005}` is three functions: 0 = name length, 1 = a 16-byte name fragment at
   a byte offset (a 59-character name takes four calls), 2 = the device-type byte. That byte
   is authoritative; guessing "Mouse" from the product string gets `G703 LS` wrong.
7. Most Logitech devices expose only `BATTERY_VOLTAGE {1001}` and a percentage has to be
   inferred; **headsets use `ADC_MEASUREMENT {1F20}` instead** (same `millivolts flags` shape,
   different flag bits: bit 0 = a battery exists, bit 1 = charging), and that call errors while
   the headset is idle rather than staying silent. `crates/hidpp/src/battery.rs`
   (`VOLTAGE_CURVE`) holds the reference curve, so our inferred percentage agrees with what
   `solaar show` prints for the same voltage — 4020 mV is 84 %, 3867 mV is 61 % — and it rounds
   to nearest rather than truncating. Every inferred value still carries `~`, and a device's own
   percentage always wins.
8. A receiver presents three hidraw nodes and only the HID++ interface answers; the other
   two fail with `EPIPE`. Group them by USB port and talk to the interface with the highest
   index.

## Continue here

```bash
export PATH=$HOME/.cargo/bin:$PATH          # rustup installed with --no-modify-path
cd ~/Projects/logi-modern
cargo test --workspace                      # 58 tests, no hardware needed
cargo clippy --workspace --all-targets      # must stay at zero warnings
cargo run --release -p periphery            # the app (window, tray, notifications)
cargo run --release --example probe -p hidpp # headless device report — the acceptance check
cargo run --example inspect -p hidpp -- /dev/hidraw0   # why is one node silent? ask it
/usr/bin/solaar show                        # ground truth to compare the probe against
```

* **Acceptance ritual**: `probe` output must agree with `solaar show` on level *and*
  millivolts for every device it reads — including the headset, which reports 3867 mV through
  ADC to both tools. The millivolts must never differ; the percentage must not either, now that
  the curve is the reference one.
* **Tests are the point, not a formality**: the battery policy's subtleties (hysteresis,
  once-per-crossing, rate limiting, snooze, no replay when notifications were off) all have
  tests with an injected clock. Add behaviour there, not in the Slint glue.

### Open decisions

1. **App name** — currently the placeholder `Periphery` (binary `periphery`, config at
   `~/.config/logi-modern/config.toml`, Slint UI title "Periphery").
2. **Which configuration feature first**: suggested order is DPI (`{2201}`) → RGB effects
   (`{8071}`) → host switching (`{1814}`) → button diversion, ascending in risk; the last one
   needs an evdev/uinput hook rather than HID++ alone.

### Known gaps (deliberate, not oversights)

* The notification's **"Show devices" action was never clicked** — the handler exists and the
  OSD with the button is photographed, but this session has no input injection. Unproven.
* The **tray icon needs pinning** in Noctalia (`[widget.tray] pinned` in
  `~/.config/noctalia-kwe/noctalia-config.toml`); no id is faked to get around it.
* The **G733 headset** reads 61 %/3866 mV through `ADC_MEASUREMENT {1F20}` while it is in use;
  while it is idle the call errors (`0x05 NOT_ALLOWED`) and the app says "Connected" with no
  reading, which is the honest answer rather than a stale number. A periodic retry for idle
  headsets is not implemented yet.
* No packaging yet (`.desktop`, systemd user unit, udev rule installation), no i18n, and the
  app is a single process — a background daemon so notifications work with the UI closed is
  still a future step (the `core`/`hidpp` split is already shaped for it).

### Throwaway material

`spikes/` holds the five experiments behind the decisions (each with a Verdict README) and
their build directories. They are disposable; delete `spikes/*/target` if the disk matters.

## Verified environment (2026-10-15)

* Rust **1.99.0** (cargo 1.99.0) at `$HOME/.cargo/bin` — installed via rustup with
  `--profile minimal --no-modify-path`, so **it is not on `PATH`**:
  `export PATH=$HOME/.cargo/bin:$PATH`. No sudo was needed and none is available.
* Distro also has: PyGObject 3.58.1, GTK 3+4, libadwaita 1.10, Qt 6 + QtQuick/Kirigami,
  Python 3.14.8, meson/ninja/gcc 16, Node 26, flatpak. No system `rustc`, no Go.
* Desktop: **KineticWE** — KWin fork + the Noctalia shell (Quickshell/QML), Wayland,
  panel widgets configured in `~/.config/noctalia-kwe/noctalia-config.toml`
  (`[widget.tray]`, `[plugins]`, `[widget.*]`). Notification server: `shelly-notifications`.
  Screenshots: `spectacle -b -n -o out.png`.
* Hidden trap: `make check` in the Python fork died on `check-yaml` because that
  pre-commit hook's env was stale and missed `ruamel.yaml`; recreating the env
  (`rm -rf ~/.cache/pre-commit/<repo>/py_env-python3.14`) fixes it. Nothing in the repo
  was wrong.

## Hardware test bench (real devices, so nothing here is theoretical)

| Device | Path | Connection | Ground truth from Solaar 1.1.20 |
| --- | --- | --- | --- |
| G703 LIGHTSPEED mouse (WPID 4086) | `/dev/hidraw8` | Lightspeed receiver `046d:c539` (`/dev/hidraw6`), device index 1 | `88% 4053mV DISCHARGING` |
| G915 TKL keyboard (WPID 408E) | `/dev/hidraw7` | Bolt receiver `046d:c547` (`/dev/hidraw1-3`), device index 1 | `95% 4122mV DISCHARGING` |
| G733 headset (WPID 0B1F) | `/dev/hidraw0` | USB direct | battery offline |

Access to `/dev/hidraw*` comes from upstream's udev rule
(`rules.d/42-logitech-unify-permissions.rules`, `TAG+="uaccess"`): a **user-session**
program can own the devices with no root and no system service.

## Spikes

Throwaway experiments; each has a `README.md` with a Verdict.

```
spikes/
  001-rust-hidpp-battery/      can Rust read battery over raw HID++ (no Python)?
  002a-slint-gui/              GUI candidate: Slint
  002b-iced-gui/               GUI candidate: iced
  002c-egui-gui/               GUI candidate: egui/eframe
  003-tray-and-notifications/  StatusNotifierItem tray + desktop notifications here
```

Every GUI candidate builds the *same* screen (three device rows with battery bars, a
details card, a settings placeholder) and reports build time, dependency count, binary
size, RSS, startup time and a screenshot — so the choice is made on measured numbers and
real pixels, not on documentation.

## Spike results (2026-10-15)

All five verdicts were reproduced by the parent session before being believed: the battery
binary was run again and compared to `solaar show` in the same minute, and every screenshot
was inspected by eye.

| Spike | Verdict | Key numbers |
| --- | --- | --- |
| 001 raw HID++ battery | **VALIDATED** | 1112 lines of Rust, 1 dependency (`libc`); G703 88 %/4048 mV and G915 TKL 94 %/4117 mV — identical to `solaar show` at the same moment; headset honestly reported as no-path; whole sweep of two receivers ≈ 4 s |
| 002a Slint GUI | **VALIDATED** | 51 s cold release build, 939 tree-lines / 342 crates, **21.9 MiB** stripped, **87 MiB** RSS, **25 ms** to window, 462 lines `.slint` + 57 lines Rust; tray/i18n/accessibility built in; system libs: fontconfig/freetype only |
| 002b iced GUI | **VALIDATED** | 37.9 s cold build, 652 / 296 crates, **15.4 MiB** stripped, **73 MiB** RSS, 76–209 ms to window, 506 lines (224 of widget code); no tray, no a11y in 0.14, no i18n |
| 002c egui GUI | **VALIDATED** | 45.7 s cold build, 736 / 256 crates, **19.0 MiB** stripped, **75 MiB** RSS, 45 ms to window, 576 lines (397 UI) — half the effort was hand-painting chrome a retained toolkit gives free; no tray/a11y/i18n |
| 003 tray + notifications | **PARTIAL** | `ksni` + `notify-rust`: notification OSD **observed on screen** with the `Show devices` action button (server advertises `actions`); tray item registers with Noctalia and its dbusmenu submenu is correct, but the bar only draws it when the item's Id is in `[widget.tray] pinned` — with its own name it goes to the hover-only drawer. Tray binary: 1.6 MiB stripped, 4.6 MiB RSS |

Caveat on the build/RSS numbers: the spikes ran concurrently and share a leaked
`CARGO_TARGET_DIR`, so two of them built into each other's `target/` (one `cargo clean`
wiped a sibling's directory). Sources were never touched and each spike re-ran its
measurement with `--target-dir` pointed at its own tree, but treat the cold-build times as
±20 %. The binary sizes, RSS, startup times and screenshots were all re-verified.

Design read from the three screenshots (parent's judgement, not the children's):
Slint produced the most finished-looking window (cards, aligned stat tiles, real switch,
consistent spacing; only the status colour for "Discharging" is off-tone). egui looks
respectable with a custom palette but its icons render as odd monochrome glyphs.
iced's prototype looks the least finished — its icon tiles read as skeleton loaders.

Also a desktop behaviour worth knowing: this KWin/KineticWE session **tiles new windows**,
so a requested 900×560 window is granted 1262×958. Every screenshot therefore has empty
space below the content; the app must be laid out to fill or to stay centred rather than
assuming the size it asked for.

