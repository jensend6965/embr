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
* **Asks every device whether it is still there, every five seconds, while the window is open.**
  A sweep on the user's interval is far too rare to notice a device *switching off* — the kernel
  says nothing when a headset is powered down behind its dongle, which is the one case the uevents
  below cannot see — so between sweeps Embr asks each device one question with deadlines short
  enough to ask it again a few seconds later: 250 ms for the liveness ping, 700 ms for the battery
  question (`Patience::Watch`). This is not the expensive kind of polling: a device that is there
  answers in single-digit milliseconds, and one that has gone costs its deadline and no more. It is
  `general.watch_devices` (on by default, a switch on the settings page) and it runs only while the
  window is open — nobody is looking at a tray icon to see a device leave, and a tray-only Embr
  stays as quiet towards the devices as it was before this existed.
* **Gives up on a device that has not answered after one timeout.** A device that is switched off
  answers nothing, and every call to it costs its whole deadline — so asking a silent device for a
  name and then for four battery features spends six timeouts learning the same silence six times.
  Two things stop that: the first question to a device is a single liveness ping on ROOT, and a
  device that says nothing to it is written off for that pass (`discover::there`); and the report
  size a device answers at (long or short HID++ reports — a device property, not a channel's) is
  remembered for the life of the process instead of being rediscovered, at the price of a timeout,
  on every pass (`transport::preferences`). Measured on this machine with a headset switched off
  and two paired devices asleep: a scheduled sweep went from 20 s to 7 s, a Rescan from 39 s to
  15 s, and a watch pass from 7 s to 2 s — with the headset itself costing 700 ms, one deadline.
* **Rescan** (the header button, and the tray menu's item of the same name) is a *thorough*
  sweep: it reads everything now and gives the devices that have not answered yet the longer
  waits (`Patience::Asked`) that a device coming back from sleep needs, so a row that said *Not
  answering* can turn into a reading instead of waiting for the next interval.
* **It uses the width it is given.** The window opens at 1000×660 (`preferred-width`/
  `preferred-height`, not `width`/`height` — a plain `width:` is a binding on the window's size, so
  the frame can grow while the layout stays at its opening width and the extra space is painted in
  the background colour) and the layout is written in terms of the space it has, so dragging it
  wider moves the panes. The device list grows to 470 px and stops, the detail card takes
  everything else, and the settings page runs its rows the full width with each control at the far
  right — the shape Solaar has when maximised. Two floors keep it honest: the list needs 380 px for
  a row's own content and the card 260 px for its tiles and the alert row, so below about 700 px
  they cannot both fit. The window's own size is logged when it changes
  (`[app] window 1646x960 (physical)`), which is the only way to tell "the layout adapted" from
  "the frame grew around a layout that did not".
* **Notices a device arriving or leaving by itself.** `embr-hidpp::Uevents` listens on the kernel's
  uevent group — a netlink socket any user may bind to, so there is no daemon to install, no
  `libudev` linkage and no helper process — and the UI thread drains it on every tick. A paired
  device waking up, a receiver being plugged in, a headset being unplugged: the kernel has already
  seen it, and the sweep that follows is the same sweep the schedule would have run. This is
  deliberately *not* polling the hardware to ask whether anything changed: on a wireless device
  every such question is a wake-up, and a battery monitor that spends the batteries is a bad one.
* **And asks again soon about a device that did not answer.** A sleeping device has not gone
  anywhere, so a sweep that could not read everything is repeated after ten seconds, then twenty,
  forty, eighty (`core::retry_interval`) — while the window is open, because that is when somebody
  is looking at it. With the watch above switched on, this ladder steps aside: a full sweep every
  ten seconds and a watch pass every five are the same question asked twice, and the sweep is the
  expensive way to ask it. With the window closed the two-minute floor stands and the uevent above
  is what catches a device really coming back; there is no point in waking a sleeping device every
  ten seconds to redraw a tooltip nobody is reading.
* **Says *Not answering* when nothing answered, even if a dongle did.** The G733's dongle replies
  to a ping in 2 ms whether the headset behind it is on or off, so a ping is not evidence about the
  device — the battery question is. A timeout in answer to *that* is silence, and silence is not
  *Connected* (`discover::answered`): it is the difference between a row that quietly kept its old
  percentage and a switched-off headset that says so.
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
* **Tray icons**: the tooltip is `Snapshot::tray_tooltip()`, and the pictograms are drawn in code as
  22×22 ARGB32 pixmaps — a battery, or a level as digits when *Tray icon shows the level* is on.
  Which icons exist is `core::tray_icons`: nothing chosen means one icon that follows the worst
  device, and a device the user picked in Settings gets one of its own, each about that device.
  They register as `embr` and `embr-<device key>`, so a bar that draws only what it has pinned
  (Noctalia) needs each id added. Left-click shows the window — on that device, for an icon that is
  about one — a *Devices* submenu lists every device (`NAME — 88% (Discharging)`), plus *Rescan* and
  *Quit*.
* **The icon set is shared between two threads, deliberately.** `tray::Icons` is held by the device
  thread (which decides which icons exist and what they say) *and* by the window, which flips their
  style the moment the switch moves. Routing a style change through the device thread made the
  switch wait out whatever sweep was in flight — measured at 827 ms with a rescan running, against
  29 ms once the window applies it itself. A presentation change has no business queueing behind
  device I/O.
* The **About** window shows the Slint `AboutSlint` widget, which the Slint licence requires.

### Threads

| Thread | Owns | Never does |
| --- | --- | --- |
| UI (main) | everything Slint, and the tray icons' *style* | open a device, read a battery |
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
