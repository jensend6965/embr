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

## How this was written

**Most of this code was written by an AI assistant, not by hand.** Jensen — the name on the
commits — directs the project: he decides what Embr should do, what it must not do, and what
"correct" means. An AI assistant (Hermes Agent, running large language models) writes the code:
the Rust, the Slint interface, the tests, the comments, and this file. It is made in conversation —
he asks for a change or reports what the app did wrong, the assistant makes it, runs the suite and
reports what it checked on the hardware.

That is worth saying plainly, because it changes what you should do with what you find here:

* **Treat it as unreviewed work by a competent stranger.** Nothing here has had a second pair of
  human eyes. The protocol layer was measured against real devices and there are tests, but the
  design and the code are the model's, and a model can be confidently wrong in ways that compile,
  pass tests and read tidily.
* **The comments and commit messages are claims, not evidence.** Every file explains itself and
  says why a thing is the way it is — written by the same process as the code it describes. Where
  it matters, check the explanation against the code.
* **What was not delegated:** the requirements, the decisions about what Embr does, the review of
  each change before it landed, and all testing on physical hardware — three Logitech peripherals
  on CachyOS with KWin and Wayland, the only environment this has ever run in. The assistant has
  never seen those devices: everything in `crates/embr-hidpp` comes from the public HID++
  documentation and from measurements taken on this machine, and the constants taken from Solaar
  are marked as such (see [Licence](#licence)).

Practically: read it before you run it, and report what breaks. HID++ has no read-only mode, so
"just trying it" means writing to your devices.

## What it does

* **Charge for every device it can reach**, headsets included. They report it through a different
  feature than mice and keyboards, and while one is idle there may be nothing to report — Embr says
  so instead of guessing.
* **Nothing invented.** A device that answered *about itself* but has no reading to give is
  *Connected*; one that said nothing to the question is *Not answering* — a dongle answering for a
  switched-off headset does not count; a percentage worked out from voltage is marked `~`, and a
  percentage a device reports itself always wins.
* **Worst battery first**, with the age of each reading beside it.
* **A window that uses the width it is given.** Dragged wider, the device list stops at a
  comfortable width and the detail card takes the rest; the settings page runs its rows the full
  width with each control at the far right, the way Solaar looks when it is maximised. Narrow
  enough and the two panes start to compete — below about 700 px they cannot both fit.
* **A cadence that matches what you are doing.** While the window is open the devices are read
  at the interval you choose; with the window closed it backs off to no more than once every two
  minutes, and reads everything the moment the window comes back. **Rescan** reads now, and gives
  a device that has been asleep longer to answer before writing it off as *Not answering*.
  Between those sweeps every device is asked one short question — are you still there — every five
  seconds, which is what makes a device switching off visible in seconds rather than at the next
  interval. It is a switch on the settings page and on by default.
* **It notices a device by itself.** Switch a headset off, pick a mouse up, switch a keyboard on,
  plug a receiver back in: a device that stops answering shows up as *Not answering* within a few
  seconds, and one that comes back does too, from the kernel — a HID device that came or went is
  announced by a uevent, which Embr hears and sweeps on at once. Nothing is polled to find out that
  nothing changed: between the short questions above, listening costs nothing.
* **Tray icon and notifications.** The icon is a battery coloured by the worst device — or the
  worst level itself, as a number, if you would rather read it than measure it. Any device can be
  given an icon of its own instead (Settings → Tray): one icon per device, each showing *its* level,
  the way a monitoring overlay puts a readout per sensor on screen. With nothing chosen there is one
  icon and it follows the lowest charge.
* **Notifications** stay off until you switch them on; then they have thresholds, hysteresis,
  one-per-crossing de-duplication, rate limiting and a snooze.
* **A threshold of its own for any device** you ask for one. A headset and a mouse do not drain at
  the same rate, so a device with its own level watches that level instead of the shared pair; set
  it next to the device it belongs to, and `Default` puts it back.
* **Close to tray**: closing the window leaves Embr watching in the background rather than
  quitting, so a low battery is still announced with nothing on screen. Turn it off in Settings
  and the close button means what it says. There is a Quit button there too, for a machine whose
  panel does not draw the tray item.
* **A settings page** for those levels, the tray icon, the close behaviour, the refresh interval
  and the notification switch, saved to `~/.config/embr/config.toml`.

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
