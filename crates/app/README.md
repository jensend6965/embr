# `periphery` — the desktop app

The window for the Logitech device monitor: the device list with battery bars, a details
card, the settings, a tray icon and desktop notifications. Binary name **`periphery`**.

It draws what `logi-core` gives it and decides nothing: the device list, the percentage text
(including the `~` that marks a value inferred from voltage), the status label, the headline,
the tray tooltip and the ages (`12s ago`) all come from the `core` crate.

## How to run it

```sh
export PATH=$HOME/.cargo/bin:$PATH
cargo run --release -p periphery       # or: ./target/release/periphery
```

No root, no daemon, no system package: access to `/dev/hidraw*` comes from upstream's udev
rule (`TAG+="uaccess"`), so a user-session program can own the devices.

## What it does

* **Refreshes straight away, then every `general.refresh_seconds`** (clamped to at least 5 s,
  default 60 s), and whenever the **Refresh** button is clicked. A sweep of two receivers
  takes seconds, so it runs on its own thread: the window stays painted, the ages keep
  ticking and the rows keep showing the previous reading while a sweep is in flight.
* **Between sweeps it holds no device open.** `Monitor::refresh()` opens what each query
  needs and closes it again before returning.
* **Snooze** on every row calls `Monitor::snooze(device.key)`; the *Battery notifications*
  switch calls `Monitor::set_notifications_enabled()` and persists the choice through
  `core`'s own config writer. Notifications are **off until the user turns them on** — that is
  the default and this app never overrides it.
* **Notifications** carry the title and body from `Alert::title_and_body()` plus a
  *Show devices* action, and are sent from their own thread, so a slow D-Bus round trip cannot
  freeze the window. While notifications are off, what a user *would* have been told is
  written to stderr instead of being silently dropped.
* **Tray icon**: tooltip = `Snapshot::tray_tooltip()`; left-click shows the window;
  a *Devices* submenu with one entry per device (`NAME — 88% (Discharging)`), plus *Refresh*
  and *Quit*.
* The **About** window shows the Slint `AboutSlint` widget, which the Slint licence requires.

### Threads

| Thread | Owns | Never does |
| --- | --- | --- |
| UI (main) | everything Slint | open a device, talk to D-Bus |
| `devices` | `core::Monitor`, the HID++ work | touch a Slint object |
| `notifications` | D-Bus (`notify-rust`) | touch the window |
| tray (`ksni`) | the StatusNotifierItem | block the UI thread |

They talk over three channels (`Command` in, `UiState`/`UiRequest` out) and never share a
Slint object or a device handle. The UI thread polls the other two on a 500 ms timer, so a
tray click reaches the window within half a second without any cross-thread UI call.

## Tray icons on KineticWE / Noctalia: the pinning caveat

This is a plain StatusNotifierItem: it takes its own bus name
(`org.kde.StatusNotifierItem-<pid>-1`), publishes `Id = periphery`, and registers with
`org.kde.StatusNotifierWatcher`. On this desktop that is **registered but not drawn**: the
Noctalia bar draws only the tray items whose application id is listed in `[widget.tray]`
`pinned`, and puts everything else in the hover-only drawer.

Nothing in this crate works around that — the workaround would be to impersonate another
application's id. To have the bar draw it, add the id to your Noctalia configuration:

```toml
# ~/.config/noctalia-kwe/noctalia-config.toml
[widget.tray]
pinned = [ "Fcitx", "Discord", "equibop", "periphery" ]
```

`periphery` is the string `Tray::id()` returns. Reopen the shell (or reload its config) for
the change to take effect.

## Not implemented yet

* **Tray-resident mode.** Closing the window exits the process, and with it the tray item and
  the monitoring loop. `general.start_hidden` is read by `core` but not acted on here.
* **Snooze feedback.** The snooze is real (the policy honours it) but the window does not yet
  say until when, and does not un-snooze.
* **Only the first notice is shown.** `Monitor::notices` is a list; the settings card shows
  the first entry.
* **No tests of its own.** All the judgement lives in `core`, which is tested; this crate is
  glue, a `.slint` file and the wiring. There is no automated test that a window appears.
* **i18n.** Every user-visible string is English and literal in the `.slint` file.
* **No icon theme, no `Icon=`.** The tray pictogram is generated (a 22×22 ARGB32 battery,
  coloured by the worst device) so no icon files have to be installed.

## Hidden debug flag

```sh
PERIPHERY_DEBUG_ALERT=1 ./target/release/periphery
```

After every sweep this sends one synthetic low-battery alert through the same code a real one
takes (`dispatch_alert` → notification thread → `notify-rust`), which is how the notification
path is exercised while notifications are off. It changes nothing else.
