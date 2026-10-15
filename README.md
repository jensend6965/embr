<p align="center">
  <img src="assets/embr.svg" alt="" width="96">
</p>

<h1 align="center">Embr</h1>

<p align="center">A battery monitor for Logitech devices on Linux.</p>

<p align="center">
  <a href="https://github.com/jensend6965/embr/actions/workflows/ci.yml"><img src="https://github.com/jensend6965/embr/actions/workflows/ci.yml/badge.svg" alt="ci"></a>
</p>

Embr shows what every attached Logitech mouse, keyboard and headset is doing — charge, voltage,
charge state and how it is connected — in a small window, a tray icon and optional desktop
notifications.

![Embr's window: three devices with battery levels, and the details card for the selected one](docs/screenshot.png)

*A device that has stopped answering says so, rather than showing a stale number.*

## What it does

* **Charge for every device it can reach**, headsets included. They report it through a different
  feature than mice and keyboards, and while one is idle there may be nothing to report — Embr says
  so instead of guessing.
* **Nothing invented.** A device that answered but has no reading to give is *Connected*; one that
  did not answer is *Not answering*; a percentage worked out from voltage is marked `~`, and a
  percentage a device reports itself always wins.
* **Worst battery first**, with the age of each reading beside it.
* **Tray icon and notifications.** The icon is a battery coloured by the worst device — or the
  worst level itself, as a number, if you would rather read it than measure it. Notifications stay
  off until you switch them on; then they have thresholds, hysteresis, one-per-crossing
  de-duplication, rate limiting and a snooze.
* **A threshold of its own for any device** you ask for one. A headset and a mouse do not drain at
  the same rate, so a device with its own level watches that level instead of the shared pair; set
  it next to the device it belongs to, and `Default` puts it back.
* **A settings pane** for those levels, the refresh interval and the two switches, saved to
  `~/.config/embr/config.toml`.

## Install

```bash
# Debian / Ubuntu
sudo apt install libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libdbus-1-dev

cargo build --release
./target/release/embr
```

Needs Rust 1.85 or newer. On Fedora and Arch the packages are `fontconfig-devel`, `freetype2`,
`libxkbcommon` and `dbus`. Embr draws its interface with the system font — Noto Sans, the family
`sans-serif` resolves to on the major distributions — and ships no fonts of its own.

## Device access

Reading a Logitech device means talking to it over `/dev/hidraw*`, which is root-only by default.
Embr does not want to run as root and installs nothing that stays running, so it ships a udev rule
that hands those nodes to whoever is logged in:

```bash
sudo install -m 0644 rules.d/70-embr-hidraw.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=hidraw
```

Without that rule Embr starts, finds no devices and shows an empty list.

**What Embr touches:**

| | |
| --- | --- |
| `/dev/hidraw*` of Logitech devices — the `046d` vendor id, the Lenovo-branded receivers, and Bluetooth-attached devices | **read and write.** HID++ has no read-only mode: the same channel carries requests and it is enough to begin a firmware update, which is why the udev rule hands access to the logged-in user only, rather than to everyone |
| `~/.config/embr/config.toml` | read at startup, written when you change a setting |
| The session D-Bus | the tray icon (`StatusNotifierItem`) and notifications (`org.freedesktop.Notifications`) |
| Nothing else | no network access, no telemetry, no root, nothing left running in the background |

## Built with

| | |
| --- | --- |
| [Slint](https://slint.dev) 1.18 | the window, the settings pane and the About dialog |
| [`ksni`](https://crates.io/crates/ksni) | the tray icon, over D-Bus |
| [`notify-rust`](https://crates.io/crates/notify-rust) | desktop notifications |
| [`serde`](https://crates.io/crates/serde) + [`toml`](https://crates.io/crates/toml) | reading and writing the config file |
| [`libc`](https://crates.io/crates/libc) | talking to `/dev/hidraw*` |

## Development

```bash
cargo test --workspace                    # no hardware and no display needed
cargo clippy --workspace --all-targets    # keep this silent
```

## Roadmap

* **Configuration** — DPI, RGB effects, host switching, then buttons.
* **Packaging** — a `.desktop` entry, a systemd user unit and an install script, so this stops
  being a `cargo build` away.
* **A background daemon**, so notifications keep working with the window closed.
* **Translations**, and a voltage curve measured from our own devices rather than the one noted
  below.

## Licence

**MIT OR Apache-2.0**, at your option — see `LICENSE-MIT` and `LICENSE-APACHE`.

Two things worth knowing:

* **Slint** is used under its Royalty-free licence, which requires the `AboutSlint` widget to be
  reachable from an About dialog. Embr's About dialog has it; please keep it there.
* **One value is derived from Solaar** (GPL-2.0-or-later): the voltage-to-percentage anchors in
  `crates/embr-hidpp/src/battery.rs`, reproduced so that a percentage inferred from voltage agrees
  with what `solaar show` prints at the same millivolts. It is stated in the code and here rather
  than left implicit, and the roadmap has the exit — measuring that curve from our own devices.
