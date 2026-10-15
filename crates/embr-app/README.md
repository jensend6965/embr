# `embr` — the desktop app

The window for the Logitech device monitor: the device list with battery bars, a details
card, a settings page, a tray icon and desktop notifications. Binary name **`embr`**.

It draws what `embr-core` gives it and decides nothing: the device list, the percentage text
(including the `~` that marks a value inferred from voltage), the status label, the alert
level the settings spin box shows, the tray tooltip and the ages (`12s ago`) all come from the
`embr-core` crate.

## How to run it

```sh
export PATH=$HOME/.cargo/bin:$PATH
cargo run --release -p embr       # or: ./target/release/embr
```

No root, no daemon, no system package: access to `/dev/hidraw*` comes from upstream's udev
rule (`TAG+="uaccess"`), so a user-session program can own the devices. `embr` also sets its
XDG app id (`slint::set_xdg_app_id("embr")`) so a `.desktop` entry and the icon theme can find
it — see the repository README for the install steps.

## What it does

* **Refreshes on a cadence that matches what you are doing.** While the window is open it sweeps
  every `general.refresh_seconds` (5–600 s, default 60 s, editable on the settings page); with
  the window closed it backs off to at least every two minutes; and it sweeps the moment the
  window comes back. `core::scheduled_interval` decides those numbers — the UI thread only
  counts. A sweep of two receivers takes seconds, so it runs on its own thread: the window stays
  painted, the ages keep ticking, and the rows keep showing the previous reading while a sweep is
  in flight.
* **Rescan** (the header button, and the tray menu's item of the same name) is a *thorough*
  sweep: it reads everything now and gives the devices that have not answered yet the longer
  waits (`Patience::Asked`) that a device coming back from sleep needs, so a row that said *Not
  answering* can turn into a reading instead of waiting for the next interval.
* **Between sweeps it holds no device open.** `Monitor::refresh_with()` opens what each query
  needs and closes it again before returning.
* **Snooze** on every row calls `Monitor::snooze(device.key)`; the *Battery notifications*
  switch calls `Monitor::set_notifications_enabled()` and persists the choice through
  `core`'s own config writer. Notifications are **off until the user turns them on** — that is
  the default and this app never overrides it.
* **Notifications** carry the title and body from `Alert::title_and_body()` plus a
  *Show devices* action, and are sent from their own thread, so a slow D-Bus round trip cannot
  freeze the window. While notifications are off, what a user *would* have been told is
  written to stderr instead of being silently dropped. The same channel carries the app's own
  one-line notices (`Notice::Text`), such as the first time the window is hidden with
  close-to-tray on.
* **Close to tray.** With `general.close_to_tray` on (the default) the close button hides the
  window and the app keeps running: the device thread keeps sweeping, the tray keeps its menu and
  a low battery is still announced. The event loop is `run_event_loop_until_quit()` for exactly
  this reason — `run_event_loop()` returns when the last window is hidden, because it counts
  windows and Slint's own tray icon, and this app's tray item belongs to `ksni` on another thread.
  With the option off, the close button quits.
* **Tray icon**: tooltip = `Snapshot::tray_tooltip()`; the pictogram is the worst battery, drawn in
  code as a 22×22 ARGB32 pixmap — a battery, or the level itself as digits when *Tray icon shows
  the level* is on. Left-click shows the window (and switches it back to the devices page);
  a *Devices* submenu with one entry per device (`NAME — 88% (Discharging)`), plus *Rescan* and
  *Quit*.
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

## Pages

The header switches between two pages. **Devices** is the list and the details card, which holds
the one setting that belongs to a device — its own low-battery level. **Settings** is everything
that belongs to the application: notifications, the shared thresholds, close-to-tray, the refresh
interval, the tray icon style, Quit, and the path of the settings file.

A page is an `if cond : Layout { ... }` branch, not a `visible:` property: an invisible layout
child is still laid out, which put the settings page *below* the devices page, out of view.

## Tray icons on KineticWE / Noctalia: the pinning caveat

This is a plain StatusNotifierItem: it takes its own bus name
(`org.kde.StatusNotifierItem-<pid>-1`), publishes `Id = embr`, and registers with
`org.kde.StatusNotifierWatcher`. On this desktop that is **registered but not drawn**: the
Noctalia bar draws only the tray items whose application id is listed in `[widget.tray]`
`pinned`, and puts everything else in the hover-only drawer.

Nothing in this crate works around that — the workaround would be to impersonate another
application's id. To have the bar draw it, add the id to your Noctalia configuration:

```toml
# ~/.config/noctalia-kwe/noctalia-config.toml
[widget.tray]
pinned = [ "Fcitx", "Discord", "equibop", "embr" ]
```

`embr` is the string `Tray::id()` returns. Reopen the shell (or reload its config) for
the change to take effect.

## Not implemented yet

* **Snooze feedback.** The snooze is real (the policy honours it) but the window does not yet
  say until when, and does not un-snooze.
* **Only the first notice is shown.** `Monitor::notices` is a list; the settings page shows
  the first entry.
* **No tests of its own but the tray's.** The judgement lives in `core`, which is tested; this
  crate is glue, a `.slint` file and the wiring. There is no automated test that a window
  appears — the tray's pixmap drawing is the exception, because it is geometry.
* **i18n.** Every user-visible string is English and literal in the `.slint` file.
* **One instance per session.** Nothing stops a second `embr` from starting and registering a
  second tray item; the first one's window would then be reachable only from its own tray item.

## Hidden debug flag

```sh
EMBR_DEBUG_ALERT=1 ./target/release/embr
```

After every sweep this sends one synthetic low-battery alert through the same code a real one
takes (`dispatch_alert` → notification thread → `notify-rust`), which is how the notification
path is exercised while notifications are off. It changes nothing else.
