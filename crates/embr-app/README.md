# `embr` — the desktop app

The window for the Logitech device monitor: a rail of devices, a page per subject, a settings
dialog, a tray icon and desktop notifications. Binary name **`embr`**.

It draws what `embr-core` gives it and decides nothing: the device list, the percentage text
(including the `~` that marks a value inferred from voltage), the status label, the pages a device
has, the alert level, the tray tooltip and the ages (`just now`, `1 min ago · 12:23 17/10/2026`) all
come from `embr-core`. The tray icons and the notifications come from `embr-desktop`, which is
toolkit-free D-Bus code.

The window is written with **GTK 4 + libadwaita** (`gtk4-rs`). It was Slint until the port finished;
`spikes/004-gtk4-gui/README.md` has what the two toolkits cost and give, measured.

## How to run it

```sh
export PATH=$HOME/.cargo/bin:$PATH
cargo run --release -p embr       # or: ./target/release/embr
```

## What it does

* **Refreshes on a cadence that matches what you are doing.** While the window is open it reads
  every device every `general.refresh_seconds` (5–600 s, default 60 s, editable in Settings); with
  the window closed it backs off to at least every two minutes; and it reads the moment the window
  comes back — with the longer waits a device that has been asleep needs. `core::scheduled_interval`
  decides those numbers; the window only counts. A sweep of two receivers takes seconds, so it runs
  on its own thread: the window stays painted, the clock beside each reading stays right, and the
  rows keep showing the previous reading while a sweep is in flight.
* **Asks every device whether it is still there, every five seconds, while the window is open.** A
  sweep on the user's interval is far too rare to notice a device *switching off* — the kernel says
  nothing when a headset is powered down behind its dongle — so between sweeps each device gets one
  short question (`Patience::Watch`, 250 ms for the liveness ping). A device that is there answers in
  single-digit milliseconds; one that has gone costs its deadline and nothing more. It is
  `general.watch_devices` (on by default, a switch in Settings) and it runs only while the window is
  open: nobody reads a tray icon to watch a device leave.
* **Gives up on a device that has not answered after one timeout**, remembers the report size each
  device answers at, and repeats a sweep that could not read everything after 10, 20, 40, 80 seconds
  (`core::retry_interval`) while the window is open. Measured on this machine with a headset switched
  off and two paired devices asleep: a scheduled sweep 20 s → 7 s, a Rescan 39 s → 15 s, a watch pass
  7 s → 2 s.
* **Rescan** (the header button, and the tray menu's item of the same name) is a *thorough* read: it
  gives a device that has not answered yet the longer waits a device coming back from sleep needs, so
  a row that said *Not answering* can turn into a reading instead of waiting for the next interval.
* **Notices a device arriving or leaving by itself.** `embr-hidpp::Uevents` listens on the kernel's
  uevent group — a netlink socket any user may bind to, so there is no daemon to install and no
  `libudev` linkage — and the UI thread drains it on every tick.
* **Says *Not answering* when nothing answered, even if a dongle did.** The G733's dongle replies to
  a ping in 2 ms whether the headset behind it is on or off, so a ping is not evidence about the
  device — the battery question is (`discover::answered`).
* **One level deeper than a flat list: device → subject.** The rail is a list of devices at a width
  of its own — a `GtkPaned` divider, so no device name and no note under it can widen the column —
  and the selected device's pages are a tab strip across the pane beside it. The column's heading is
  the devices it has and the worst reading among them (`3 DEVICES · LOWEST ~47%`), with when the
  readings were taken beside it; the window's own title is the app's name. The page list comes from
  each device's own `FEATURE SET {0001}` (`core::pages_for`, read once per device and kept), so the
  strip cannot offer the G913 keyboard a DPI page it has no feature for, and it offers nothing whose
  controls are not written: `FeaturePage::offered` keeps the five roadmap pages out of the window
  (Jensen's 2026-10-17 ask — "we only able to control battery monitoring"), while `core::pages_for`
  still reads and keeps the whole inventory for the roadmap and the log. Two things are said out
  loud rather than hidden: a device that has not answered about its features yet says **asking this
  device what it can be asked about**, is asked again on `core`'s ladder (10 s, then 20 s, three
  attempts), and then says **this device did not answer** with Rescan beside it; and one that
  answered only *part* of its feature list says **Some features did not answer** — a subset is not
  the same thing as a device with no lighting.
* **The tab you pick stays picked.** A reading arrives every few seconds and the window is redrawn
  on every one of them, so the redraw is a *pull*: it writes values into widgets that already exist
  and never chooses anything the user chooses. Which tab is showing is read back from the strip, and
  a rebuild — a feature set arriving, a device's trouble changing — puts you back on the tab you
  were on, per device. Measured on the running window: a click on Lighting was still Lighting twenty
  seconds and four watch passes later, and switching to another device and back came back to
  Lighting rather than to Battery.
* **A glyph per device in the rail, drawn rather than looked up.** Each row carries a mouse, a
  keyboard, a headset (or a trackball, touchpad, keypad, remote) in front of its name. The shapes are
  cairo primitives in `widgets::device_glyph`, not an icon name: an icon name is a bet on the
  desktop's icon theme, and on this machine that bet is lost — GTK resolves the theme as `"gnome"`,
  has only a handful of symbolic names, and draws the rest as its missing-image box. Drawn shapes
  take the row's own colour, so they dim with a device that is not answering and follow light, dark
  and the accent with no second set of assets.
* **Two controls write**, both on the Battery page and both through `core`: the device's own
  low-battery level (a spin box, with *Shared* to put it back on the shared thresholds) and the
  snooze. A number the user is moving is protected from a snapshot taken before their edit — the page
  holds still until the snapshot that confirms it arrives.
* **Settings** is a dialog behind the header's gear: the tray icon (five styles, with the tray's own
  pixmaps as the previews, and a text colour for the one style that has text), one icon per device,
  notifications (the switch, the thresholds it uses, the snooze period), the cadence and the watch
  switch, close-to-tray, Quit, and where the file is written. Notifications are **off until you turn
  them on** — that is the default and this app never overrides it. While they are off, what a user
  *would* have been told goes to stderr instead of being silently dropped.
* **Close to tray.** With `general.close_to_tray` on (the default) the close button hides the window
  and the app keeps running: the device thread keeps sweeping, the tray keeps its menu and a low
  battery is still announced. With it off, the close button quits.
* **Tray icons**: the tooltip is `Snapshot::tray_tooltip()`, and the pictograms are drawn in code as
  22×22 ARGB32 pixmaps — one function per style in `embr-desktop::tray`, chosen by `ui.tray_style`
  (`core::TrayStyle`): a battery, the same battery standing on end, the level as digits, a ring whose
  arc is the level, and *level and status* — the battery with a bolt while charging and a mark for a
  device that reports a fault. Every shape that is not text comes from one template, drawn on integer
  coordinates so it comes out crisp rather than smeared, and the level is how much of the tube is
  filled, which needs no colour. With nothing chosen there is one icon, and it follows the device with
  the least charge; a device you pick in Settings gets one of its own. They register as `embr` and
  `embr-<device key>`, so a bar that draws only what it has pinned needs each id added. Left-click
  shows the window — on that device, for an icon that is about one — and the menu lists every device
  (`NAME — 88% (Discharging)`), plus *Rescan* and *Quit*.
* **The icon set is shared between two threads, deliberately.** `embr-desktop::tray::Icons` is held by
  the device thread (which decides which icons exist and what they say) *and* by the window, which
  flips their style the moment the picker moves. Routing a style change through the device thread made
  the switch wait out whatever sweep was in flight — measured at 827 ms with a rescan running, against
  29 ms once the window applied it itself. A presentation change has no business queueing behind
  device I/O.

### Threads

| Thread | Owns | Never does |
| --- | --- | --- |
| UI (main) | every widget, and the tray icons' *style* | open a device, read a battery, touch D-Bus |
| `devices` | `core::Monitor`, the HID++ work, which tray icons exist | touch a widget |
| `notifications` | D-Bus (`notify-rust`) | touch the window |
| tray (`ksni`) | the StatusNotifierItem | block the UI thread |

They talk over four channels (`Command` and `UiEvent` in, `State` out, `TrayAction`/`Notice` across)
and never share a widget or a device handle. The UI thread drains them on a 250 ms
tick, so a tray click reaches the window without any cross-thread UI call.

## Tray icons on KineticWE / Noctalia: the pinning caveat

This is a plain StatusNotifierItem: it takes its own bus name
(`org.kde.StatusNotifierItem-<pid>-1`), publishes `Id = embr`, and registers with
`org.kde.StatusNotifierWatcher`. On this desktop that is **registered but not drawn**: the Noctalia
bar draws only the tray items whose application id is listed in `[widget.tray]` `pinned`, and puts
everything else in the hover-only drawer.

Nothing in this crate works around that — the workaround would be to impersonate another
application's id. To have the bar draw it, add the id to your Noctalia configuration:

```toml
# ~/.config/noctalia-kwe/noctalia-config.toml
[widget.tray]
pinned = [ "Fcitx", "Discord", "equibop", "embr" ]
```

`embr` is the string `Tray::id()` returns. Reopen the shell (or reload its config) for the change to
take effect.

## Not implemented yet

* **The feature pages beyond Battery and Device are not in the window at all.** DPI, lighting,
  profiles, buttons and host switching are roadmap work; a page appears only once its controls exist
  (`FeaturePage::offered`), because a tab whose contents are a sentence about what is missing is a
  promise rather than a page. What the hardware advertised is still read, kept (`core::pages_for`)
  and logged — that is what the roadmap is built from — it is just not offered as a tab.
* **A partial feature read is not retried by itself.** A device that answered *nothing* is asked
  again on its own (three times over a minute) and then says it did not answer; one that answered
  half its feature list is not re-read until Rescan.
* **Snooze feedback.** The snooze is real (the policy honours it), but the window does not say until
  when, and does not un-snooze.
* **Only the first notice is shown** — `Monitor::notices` is a list.
* **i18n.** Every user-visible string is English and literal in the Rust.
* **One instance per session.** Nothing stops a second `embr` from starting and registering a second
  tray item.
* **No tests of its own.** The judgement lives in `core`, which is tested; this crate is glue and a
  window, and there is no automated test that a window appears. The tray's pixmap drawing *is*
  tested, in `embr-desktop`, because it is geometry.

## Hidden debug flags

```sh
EMBR_DEBUG_ALERT=1 ./target/release/embr        # one synthetic low-battery alert per sweep
EMBR_DEBUG_SETTINGS=1 ./target/release/embr     # open the settings dialog at startup
EMBR_DEBUG_PAGE=lighting ./target/release/embr  # open a page that would otherwise need a click
EMBR_DEBUG_DEVICE=G913 ./target/release/embr    # open a device that is not the worst one
EMBR_DEBUG_WARNINGS='slot 3: no reply' ./target/release/embr   # declare a machine-wide problem
```

`EMBR_DEBUG_PAGE` waits for the feature set to arrive and opens the page only if the device has it —
a page the hardware cannot serve is not a page, and a page this build has no controls for is not
offered by any device, so the flag says which of the two it is instead of waiting for ever. The
other four exist because those parts are otherwise click-only, and nothing on this desktop can
inject a click. `EMBR_DEBUG_WARNINGS` takes
semicolon-separated sentences, one row each, and is the only way to look at the **Problems** group
under the tabs without a receiver misbehaving in the middle of a sweep — that group's rows are drawn
from a list this app keeps, and getting that bookkeeping wrong once froze the window. None of them
changes anything else.

## Verifying the tray without a bar that draws it

The item is a plain StatusNotifierItem, and its pixmap is exactly what a panel would draw:

```sh
pid=$(pgrep -x embr | head -1)
qdbus6 org.kde.StatusNotifierWatcher /StatusNotifierWatcher \
  org.freedesktop.DBus.Properties.Get \
  org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems   # lists org.kde.StatusNotifierItem-<pid>-1
busctl --user --json=short get-property "org.kde.StatusNotifierItem-$pid-1" \
  /StatusNotifierItem org.kde.StatusNotifierItem IconPixmap     # 22x22 ARGB
```

`--json=short` is what makes it parseable: the plain output is an `a(iiay)` dump whose byte array is
not hex. The bytes are ARGB in network order.

## Building it

GTK 4 and libadwaita 1 must be installed — on Debian/Ubuntu `libgtk-4-dev libadwaita-1-dev
libdbus-1-dev pkg-config`, on Arch `gtk4 libadwaita dbus`. The version features in the root
`Cargo.toml` are mandatory and not cumulative: without `v1_4` the `adw` prelude has no
`ExpanderRowExt`, and a method that exists in C is invisible in Rust.
