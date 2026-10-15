# Spike 003 — StatusNotifierItem tray icon + desktop notification on KineticWE / Noctalia

**Date:** 2026-10-15 · **Host:** KineticWE (KWin + Noctalia 5.2.0), Wayland (`wayland-0`), Rust 1.99.0
**Work dir:** `<repo>/spikes/003-tray-and-notifications`

---

## Verdict: PARTIAL

| Capability | Result |
|---|---|
| Desktop notification (`notify-rust` → `org.freedesktop.Notifications`) | **VALIDATED** — observed on screen with title, body **and** the "Show devices" action button |
| StatusNotifierItem tray icon (`ksni`) | **PARTIAL** — the item registers with the shell and **is rendered in the bar**, but *only when its SNI `Id` matches an entry in Noctalia's `[widget.tray].pinned` allowlist*. Under the app's own `Id` the item is accepted by the shell (`[tray] tray item registered …`) but is **not drawn in the bar**; it is kept in Noctalia's hover-only `tray-drawer`, which is the default location for un-pinned tray items. |

Both the tray icon and the notification were **observed on screen simultaneously** in
`evidence/d1-discord-t2.png` (tray `Id` = `Discord`, i.e. a name already in the pinned list —
see "Findings" for why that was necessary and what the real fix is).

**Critical finding for the project:** a Solaar rewrite cannot assume its tray icon appears on
this desktop just by registering a StatusNotifierItem. Noctalia's bar tray is allowlist-gated
by `[widget.tray].pinned`; the shipping app must either document a one-line config addition or
drive Noctalia's own "pin" action (the shell logs `tray pin added token='…'` for a *live* item).

---

## What was built

One small Rust binary that does all three steps in order and logs one line per step:

1. **Tray (ksni 0.2.2 `TrayService`)** — `Id`/`Title` = `Solaar`, category `Hardware`, status `Active`,
   a **generated 22×22 ARGB32 pixmap** (green battery; no icon-theme dependency), a tooltip
   `Solaar — G703 88%, G915 95%`, a left-click `Activate` that runs `notify-send`, a middle-click
   handler, and a `com.canonical.dbusmenu` menu: `Show devices` / separator / **`Devices` submenu
   with one entry per fake device** (`G703 LIGHTSPEED — 88%`, `G915 TKL — 95%`, `G733 — offline`,
   disabled) / separator / `Quit spike`. Stays up 40 s by default, then exits cleanly.
2. **Notification (notify-rust 4.18)** — queries `GetServerInformation`/`GetCapabilities` from inside
   the binary, then sends summary `Solaar battery`, body
   `G703 LIGHTSPEED Wireless Gaming Mouse: 20% (discharging)`, urgency `normal`,
   timeout 6 s, and action `("show", "Show devices")` because the server advertises `actions`.
3. **Observation** — screenshots while tray + OSD are up (see evidence below).

### Run it

```bash
export PATH=$HOME/.cargo/bin:$PATH
cd <repo>/spikes/003-tray-and-notifications
cargo build --release
./target/release/solaar-tray-spike                      # tray + notification, 40 s, then clean exit
# harnesses used for the evidence below:
./final_ab.sh d1-discord                                # tray+OSD capture, default env
SOLAAR_TRAY_ID=Discord ./final_ab.sh mytest             # tray Id override
SOLAAR_SKIP_NOTIF=1 SOLAAR_TRAY_STAY_SECS=15 ./observe.sh mytest   # tray-only, with noctalia log diff
# env knobs: SOLAAR_TRAY_ID, SOLAAR_TRAY_ICON_NAME, SOLAAR_TRAY_STAY_SECS, SOLAAR_SKIP_NOTIF
```

---

## Evidence

### 1. Who owns the tray host and the notification server (verbatim)

```
$ busctl --user list | grep -i -E 'statusnotifier|notif'
:1.11                                             1424 shelly-notifica jensend :1.11  user@1000.service -  -
org.equicord.equibop.StatusNotifierItem          1821 electron        jensend :1.46  user@1000.service -  -
org.freedesktop.Notifications                    1436 noctalia-kwe    jensend :1.26  session-4.scope   4  -
org.kde.StatusNotifierItem-1424-1                1424 shelly-notifica jensend :1.11  user@1000.service -  -
org.kde.StatusNotifierWatcher                    1436 noctalia-kwe    jensend :1.26  session-4.scope   4  -
org.shellyorg.Notifications                      1424 shelly-notifica jensend :1.11  user@1000.service -  -

$ gdbus introspect --session --dest org.kde.StatusNotifierWatcher --object-path /StatusNotifierWatcher
      readonly b IsStatusNotifierHostRegistered = true;
      readonly i ProtocolVersion = 0;
      readonly as RegisteredStatusNotifierItems = [':1.45/StatusNotifierItem', ':1.46/StatusNotifierItem'];
```

`org.kde.StatusNotifierWatcher` **and** `org.freedesktop.Notifications` are both owned by
**noctalia-kwe (pid 1436)**. `shelly-notifications` (pid 1424) only owns its private
`org.shellyorg.Notifications` name and registers its own tray item — it is **not** the
`org.freedesktop.Notifications` owner on this session, contrary to the task brief.

### 2. Notification capabilities (verbatim)

```
$ gdbus call --session --dest org.freedesktop.Notifications \
    --object-path /org/freedesktop/Notifications --method org.freedesktop.Notifications.GetCapabilities
(['actions', 'activation-token', 'body', 'persistence', 'inline-reply'],)

$ ... --method org.freedesktop.Notifications.GetServerInformation
('noctalia', 'noctalia-dev', '5.2.0', '1.2')
```

The server **does** advertise `actions`; the action button was therefore requested and is visible
in the screenshot. The binary's own log agrees (line 11–12 of `evidence/binary-log-run1.txt`).

### 3. The binary's own log output

`evidence/binary-log-run1.txt`:

```
[0] ENV   pid=83827 WAYLAND_DISPLAY="wayland-0" XDG_CURRENT_DESKTOP="KineticWE"
[0] ENV   tray id="solaar" icon_name="" stay=40s
[1] TRAY  registering StatusNotifierItem (title="Solaar", 22x22 ARGB32 pixmap, submenu)
[1] TRAY    device: G703 LIGHTSPEED — 88 (discharging)
[1] TRAY    device: G915 TKL — 95 (discharging)
[1] TRAY    device: G733 — 0 (offline)
[1] TRAY  TrayService::spawn() returned; D-Bus object owned as org.kde.StatusNotifierItem-83827-1
[1] TRAY  ksni Handle acquired (update()/shutdown() available): tray thread is running
[2] NOTIF querying server capabilities
[2] NOTIF GetServerInformation -> name="noctalia" vendor="noctalia-dev" version="5.2.0" spec="1.2"
[2] NOTIF GetCapabilities -> ["actions", "activation-token", "body", "inline-reply", "persistence"]
[2] NOTIF server advertises 'actions' = true
[2] NOTIF sending: summary="Solaar battery" body="G703 LIGHTSPEED Wireless Gaming Mouse: 20% (discharging)" urgency=normal timeout=6s action=("show", "Show devices")
[2] NOTIF dispatched on background thread; staying alive 40s for observation
[3] OBSERVE  waiting 40s — screenshot window is NOW
[2] NOTIF notification accepted by the server (Notify() returned an id)
[3] EXIT   clean exit after 40s
```

### 4. Shell-side confirmation that the item was accepted (Noctalia debug log)

Noctalia writes to `~/.local/share/noctalia-kwe.log`; `noctalia-kwe msg log-level-set debug` was
used to raise the level (restored to `info` afterwards). `evidence/idA.meta` / `run2.meta`:

```
05:56:08.423 [DBG] [tray] tray item registered id=org.kde.StatusNotifierItem-109300-1/StatusNotifierItem bus='org.kde.StatusNotifierItem-109300-1' path='/StatusNotifierItem'
05:56:11.422 [DBG] [tray] tray probe failed bus='org.kde.StatusNotifierItem-109300-1' path='/org/ayatana/NotificationItem' err=[org.freedesktop.DBus.Error.NoReply] Method call timed out
05:56:23.923 [DBG] [tray] item unregistered: org.kde.StatusNotifierItem-109300-1/StatusNotifierItem
```

The item registered with the host, was held for the whole run, and was released on exit. The
`/org/ayatana/NotificationItem` probe failure is expected (ksni implements only the KDE SNI spec).

### 5. The SNI item's own properties (verbatim, live)

```
$ gdbus call --session --dest org.kde.StatusNotifierItem-114331-1 --object-path /StatusNotifierItem \
    --method org.freedesktop.DBus.Properties.Get org.kde.StatusNotifierItem ToolTip
(<('', [(22, 22, [byte 0x00, … 0xff, 0xdc, 0xf0, 0xdc, 0xff, 0x3c, 0xbe, 0x5a, …])], 'Solaar', 'Solaar — G703 88%, G915 95%')>,)
  Title = 'Solaar'   Id = 'equibop'   Category = 'Hardware'   Status = 'Active'   IconName = ''   WindowId = 0
```

(Note the pixmap bytes: `0xff,0x3c,0xbe,0x5a` = A=255 R=60 G=190 B=90 — the green battery body.)

### 6. The dbusmenu layout (verbatim) — submenu with one entry per fake device

```
$ gdbus call --session --dest <owner> --object-path /MenuBar \
    --method com.canonical.dbusmenu.GetLayout 0 -- -1 "[]"
(uint32 0, (0, {'children-display': <'submenu'>}, [
   <(1, {'label': <'Show devices'>}, @av [])>,
   <(2, {'type': <'separator'>}, @av [])>,
   <(3, {'label': <'Devices'>, 'children-display': <'submenu'>}, [
        <(4, {'label': <'G703 LIGHTSPEED — 88%'>}, @av [])>,
        <(5, {'label': <'G915 TKL — 95%'>}, @av [])>,
        <(6, {'label': <'G733 — offline'>, 'enabled': <false>}, @av [])>])>,
   <(7, {'type': <'separator'>}, @av [])>,
   <(8, {'label': <'Quit spike'>}, @av [])>]))
```

### 7. Screenshots — what is visibly in them

| Path | What is visibly in it |
|---|---|
| `evidence/d1-discord-t2.png` | **The money shot.** Top-right: the notification OSD reading **"Solaar battery" / "G703 LIGHTSPEED Wireless Gaming Mouse: 20% (discharging)" / [Show devices] / "Solaar"** with a close ×. Top bar: a **green battery icon** between the keyboard-layout icon and the notification bell (`Id="Discord"`). |
| `evidence/ZOOM-traybar.png` | 6× crop of the bar segment `…49°C 46°C > ˅ ⌨ [green battery] 🔔• > 05:58…` showing the pixmap-rendered tray icon. |
| `evidence/ZOOM-notification.png` | 2.5× crop of the OSD: title, body, the wide **"Show devices"** action button, app name `Solaar`, and the `battery-caution` themed icon. |
| `evidence/idA-bar.png` | `Id="solaar"` run, tray-only: **no** icon in the bar (0 green pixels in the bar strip). |
| `evidence/idB-bar.png`, `evidence/d2-fcitx-t2.png` | `Id="Discord"` / `Id="Fcitx"` (both in the pinned list): icon present. |
| `evidence/00-baseline.png` | Desktop before any spike: 0 green pixels in the bar strip. |

Pixel proof (exact icon colour `(60,190,90)`, unblended) in the top bar strip (`y < 64`):

| screenshot | tray `Id` | icon pixels in bar |
|---|---|---|
| `00-baseline.png` | — (nothing running) | 0 |
| `idA-bar.png` | `solaar` | 0 |
| `idC-bar.png` | `equibop` | 0 |
| `idB-bar.png` | `Discord` | 64 (x 1288–1585) |
| `d1-discord-t2.png`, `d1-discord-t4.png` | `Discord` (+ notification) | 36 (x 1578–1585) |
| `d2-fcitx-t2.png`, `d2-fcitx-t4.png` | `Fcitx` | 32 (x 1578–1585) |

---

## Findings & fixes tried (3 concrete attempts)

**Root cause found in Noctalia's config and binary:**

```toml
# /home/jensend/.config/noctalia-kwe/noctalia-config.toml
[widget.tray]
detached_panel = true
drawer = true
match_adjacent_spacing = true
pinned = [ "Fcitx", "Discord", "equibop" ]      # <-- allowlist
```

`/usr/bin/noctalia-kwe` contains `TrayDrawerPanel`, `drawer_columns`, `drawer_item_size`,
`tray pin added token='{}' for id={} …` and `tray widget icon id={} source=pixmap size={}x{} failed-to-load`
— i.e. pixmaps are supported, and un-pinned items are routed to a drawer panel.

| # | Fix attempted | Result |
|---|---|---|
| 1 | Set the SNI `Id` to a name already in `pinned` (`Discord`, then `Fcitx`) | **WORKED** — green pixmap icon rendered in the bar at x≈1578–1585, reproducible across 3 runs. ⇒ real fix: add the app's `Id` to `[widget.tray].pinned` (or use Noctalia's own pin action, which logs `tray pin added token=…`, i.e. pinning binds to a *live* item). |
| 2 | Use a different pinned token already in the list but belonging to a *running* app (`equibop`) | **FAILED** — 0 icon pixels. The shell's pin for `equibop` had been revoked at runtime (`[tray] tray pin removed token for id=:1.45 …`, and a real `:1.46` equibop item is registered), so a stale allowlist name alone is not enough; the pin token must match a live, un-duplicated item. |
| 3 | Open Noctalia's overflow tray drawer via IPC (`noctalia-kwe msg panel-open tray-drawer`) while the item is up | **INCONCLUSIVE** — the panel state opens (`[panel] panel manager: opened "tray-drawer"`, `msg status` → `"activePanelId": "tray-drawer"`) but it renders nothing visible in the screenshot and auto-closes after ~1–10 s; only the named panels `clipboard, control-center, launcher, polkit, session, setup-wizard, test, …, tray-drawer, wallpaper` exist. The drawer appears to need a real click anchor (binary strings: `tray menu: missing popup anchor context (parent={}, output={}, serial={})`). **No input-injection tooling exists on this session** (`ydotool`, `dotool`, `wtype`, `xdotool`, `wlrctl`, `kdotool` all absent), so the click could not be synthesized. |

Hypotheses disproved along the way:
* **Not an icon-format problem.** The generated ARGB32 pixmap renders correctly byte-for-byte
  (`0xff,0x3c,0xbe,0x5a`), so no icon-theme dependency or `icon_name` is required.
  (`IconName` was empty in every run and the pixmap still rendered.)
* **Not a tooltip/title problem.** `ToolTip` and `Title` are correctly exposed (evidence 5).
* **Not a "shell didn't see it" problem.** The shell explicitly logs registration (evidence 4).

**Tooltip honesty note:** the tooltip was verified at the D-Bus property level (`'Solaar — G703 88%, G915 95%'`);
it was **not** observed rendered on screen, because that requires hovering the icon and no
input-injection tooling is available on this session.

---

## Measurements

| Metric | Value |
|---|---|
| Cold `cargo build --release` (after `cargo clean`, wall clock) | **29.77 s** (also 26.20 s for a build that failed only in the final crate; `cargo clean` removed 412 MiB) |
| Incremental release rebuild | 8.3–19.7 s |
| Binary size, unstripped | **4,623,120 B** (4.41 MiB) |
| Binary size, after `strip` | **1,640,848 B** (1.56 MiB) — 64.5 % smaller |
| `cargo tree --edges normal \| wc -l` | **180** lines |
| Direct dependencies | **2** — `ksni 0.2.2`, `notify-rust 4.18.2` |
| `ldd` shared-object deps | 6 (incl. `libdbus-1.so.3`, `libsystemd.so.0`) |
| Resident memory with the tray item up (`VmRSS` from `/proc/<pid>/status`) | **4,648 kB** (run 1, stay=40 s) / **4,628 kB** (run 2) |
| `VmSize` / `Threads` with tray item up | 344,724 kB / **6** |
| Notification server | `noctalia` (Noctalia 5.2.0, spec 1.2) — owns `org.freedesktop.Notifications` |

The binary is heavier than ideal because `notify-rust` pulls all of `zbus`/tokio-lite alongside
`ksni`'s `dbus`/`dbus-tree` (two independent D-Bus stacks in one process). A shipping Solaar could
use a single D-Bus stack (drop `notify-rust` and send notifications over zbus directly) and cut
most of the 180-line tree.

---

## Files

* `src/main.rs` — the spike binary (all three steps)
* `Cargo.toml` — `ksni 0.2.2`, `notify-rust 4.11`; release profile `opt-level="z"`, LTO, `strip=false`
* `observe.sh`, `final_ab.sh`, `probe_drawer.sh`, `probe_pinned.sh`, `probe_final.sh`, `probe_menu.sh` — evidence harnesses
* `evidence/` — logs, D-Bus dumps, screenshots (15 MB); `evidence/dbus-evidence.txt` collects all verbatim D-Bus output
