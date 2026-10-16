# Spike 004 — the same window in GTK 4 + libadwaita

**Question this spike answers:** if Embr's UI were written with GTK instead of Slint, what would it cost
(build, footprint, lines), what would it give us as the feature set grows (DPI, lighting, profiles,
buttons), and does the desktop's own theme come along for free?

**Answer: yes to all three, with a measured price.** This spike was the evidence for the decision
while it was still a decision. It has since been taken: the port is done, `crates/embr-app` is a GTK
4 window, and Slint is out of the tree — so read this as the measurement behind that move rather than
as a proposal, and read the numbers below as "as measured then", on the prototype.

```sh
export PATH=$HOME/.cargo/bin:$PATH
cd spikes/004-gtk4-gui
./run.sh                 # build + run the prototype (or: cargo build --release && ./target/release/gtk-spike)
EMBR_SPIKE_COLOUR=1 ./run.sh   # also opens the stock colour picker four seconds in
EMBR_SPIKE_AT_4S=libadwaita ./run.sh   # asks libadwaita for the light scheme at t+4s
```

Environment variables the prototype reads: `EMBR_SPIKE_NO_FLIP=1` (don't flip the scheme at t+6s),
`EMBR_SPIKE_AT_STARTUP=`/`EMBR_SPIKE_AT_4S=` `libadwaita|gtk-theme`, `EMBR_SPIKE_COLOUR=1`.

## What was built

The devices page from `design/ui-redesign.html`, in real widgets — the rail with the selected device's
features under it (`AdwExpanderRow` + `AdwActionRow`), the feature page as
`AdwPreferencesPage`/`AdwPreferencesGroup`/`AdwActionRow`, and then one group per thing the roadmap
needs, so the claim "the toolkit already has it" could be checked rather than assumed:

| Roadmap control | Widget it turned out to be |
|---|---|
| DPI slots | `AdwActionRow` + `GtkScale` + `GtkSwitch` |
| Report rate / mode | linked `GtkToggleButton`s, `AdwComboRow` |
| Brightness / numbers | `AdwSpinRow` |
| **Colour picker** | `GtkColorDialogButton` + `GtkColorDialog` — stock, photographed in `evidence/02` |
| Zone map | `GtkDrawingArea` — the one thing that is custom work in *any* toolkit |
| Key capture | `GtkEventControllerKey` on a button |
| Profile name / list | `AdwEntryRow`, `GtkListBox` + add/remove (wired in ~10 lines) |
| "Written to the device" feedback | `AdwToastOverlay` + `AdwToast` |
| Appearance (System/Dark/Light) | `AdwStyleManager`, ~10 lines |

`src/main.rs` is **~490 lines of Rust for one and a half screens** — the honest cost signal: GTK's
widgets are free, the code to arrange them is not.

## Measurements (this machine, 2026-10-16, GTK 4.24.1 · libadwaita 1.10 · glib 2.90 · rustc 1.99)

| # | Metric | GTK 4 + libadwaita | Slint (Embr today / spike 002a) |
|---|---|---|---|
| 1 | Cold release build (`cargo clean`) | **32.3 s** | 51 s |
| 2 | Crates in the dependency graph | **75** | 342 |
| 3 | Stripped binary | **772 KB** | 22 MB |
| 4 | Direct shared libraries (`DT_NEEDED`) | **9** — `libgtk-4.so.1`, `libadwaita-1.so.0`, gio/glib/gobject, cairo, libc | base desktop only, deliberately no GTK/Qt |
| 5 | exec → window presented | 74–84 ms | **27 ms** |
| 6 | RSS idle | **79.2 MiB** | 96.9 MiB |
| 7 | VSZ idle | **1183 MiB** | 1445 MiB |
| 8 | CPU after 22 s idle | 0 s | 0 s |
| 9 | Threads / open fds | **11** / 23 | 26 / 21 |

Numbers 6–9 are not like-for-like: the Slint app is doing real device work (HID++ sweeps, tray,
notifications) while this prototype draws hardcoded data. Read the pair as "same ballpark".

Reproduce with `scripts/run-perf.sh` (runs both, 22 s each) and `scripts/shot-gtk-roadmap.sh`.

Remeasured when this spike was filed into the repo (the roadmap groups below were added after the first
pass): cold release build **35.9 s**, binary **837 KB**. The table's 32.3 s / 772 KB are the first cold
build of the smaller prototype; both numbers are real, the difference is the extra code plus noise.
The spike is kept out of the workspace by the root `Cargo.toml`'s `exclude` list — with a glob it does
not work, which is why the paths there are spelled out one per spike.

## The theme: what GTK gives for free, and what it does not

* **Free, verified:** the style manager reports `color-scheme=Default dark=true`,
  `system-supports-color-schemes=true`, and `accent: Purple -> #9141AC`. That accent comes from the
  *portal* (`org.freedesktop.appearance accent-color` = `#BD93F9`), **not** from
  `gsettings org.gnome.desktop.interface accent-color`, which says `blue` here. Every switch, the
  suggested-action button, selection and focus rings follow it without a line of our code.
* **Not shown, and not claimed:** the visible dark↔light swap. Asking for `ForceLight` from inside the
  app flips `is_dark` while the window keeps every pixel (3841 of 1.24 M differ — noise);
  `GTK_THEME=Adwaita:light` and `GTK_THEME=HighContrast` are both ignored (the theme name stays
  `Adwaita`/`Adwaita-empty`); `gtk-application-prefer-dark-theme` is deprecated as of GTK 4.20; and a
  *sandboxed* GSettings keyfile (see `scripts/run-gsettings-sandbox.sh`, which leaves the real desktop
  alone) changes nothing either. The only lever is the desktop's own value — published here by
  `org.freedesktop.impl.portal.desktop.kwe` — so **the experiment that decides this is flipping the
  desktop's scheme with the spike running, and putting it back**.
* **Found on the way:** GTK 4 styling on this machine is already half broken — every GTK 4 app logs
  `Failed to import …/libadwaita.css` and `…/libadwaita-tweaks.css`
  (`~/.config/gtk-4.0/gtk.css` imports three files, two of which do not exist) and
  `~/.config/gtk-4.0/settings.ini` names `adw-gtk3-dark`, a *GTK 3* theme GTK 4 ignores.

## Traps that cost a build cycle each

* `gtk4`'s library name is `gtk4`, and `libadwaita` must be aliased (`adw = { package = "libadwaita" }`)
  or named `libadwaita` — the docs' `use gtk::…` does not compile as-is.
* **Version features are not optional.** Without `v1_4` the prelude quietly lacks `ExpanderRowExt`, so
  `expander.add_suffix(..)` resolves to `ActionRowExt` and fails with *the trait `IsA<ActionRow>` is not
  implemented*; `SwitchRow`/`ToolbarView` need `v1_4`, `ColorDialogButton` needs `v4_10`. Both crates
  are therefore enabled up to the newest version the crate knows (`v1_1..v1_10`, `v4_4..v4_24`).
* **An `AdwActionRow` title is parsed as markup**: `DPI & sensitivity` rendered as *nothing* until it
  was written `DPI &amp; sensitivity`.
* `gtk4::Settings::default()` returns an `Option`, and `ColorDialogButton` is a `Widget`, not a
  `Button` — there is no `emit_clicked`; `activate()` opens the dialog.

## Evidence

`evidence/` (untracked on purpose — `.gitignore` drops `spikes/*/evidence/`):

* `01-window-dark-and-picker.png` — the rail with the selected device's features, the feature page, and
  the stock colour picker open over it, with an `AdwToast` from the self-driven "Add profile" click.
* `02-colour-dialog.png` — `GtkColorDialog`, 534×360, i.e. the control Slint has no answer for.
* `03-slint-vs-gtk.png` — the Slint window as it shipped, above the same design in GTK widgets.
  Kept because the Slint window is no longer in the tree: this is what the port replaced.
* `04-window-battery-page.png` — the Battery/DPI page at 1240 px.

## Verdict — and what is still open

**Recommendation: port the UI to GTK 4 + libadwaita in stages, keeping `embr-core`/`embr-hidpp`
untouched.** The toolkit never touches the app's brain, so the port is confined to the presentation
layer (~1 557 lines of `.slint` + ~1 100 of glue), and the exit stays open: a thin shell can move again.

Still open:

1. **Go / no-go on GTK**, and `gtk4-rs` + libadwaita versus Relm4 on top of it. *(Answered: go,
   with plain `gtk4-rs` + libadwaita — see `crates/embr-app/`, which is what it became.)*
2. **The theme test** this spike could not run without touching his desktop (flip it, photograph, flip
   it back) — the one unproven half of "follow the system theme".
3. **Fix the machine's GTK 4 styling** first (the two broken imports and the GTK 3 theme name), or the
   new app inherits the same half-broken state.
4. **The Adwaita clamp**: libadwaita centres preference content in a readable ~600 px column, which is
   the opposite of the "rows run the full width, control at the far right" decision made for the
   current window. One of the two has to give.

The staged plan, in order — and what became of it: this spike → the machine's GTK styling (his
`gtk.css` had two `@import`s of files that do not exist; repaired) → window + rail + features + a
Battery page against real devices (done) → settings, close-to-tray, tray styles and notifications at
parity (done) → **the Slint layer and the dependency deleted (done)** → the redesign, with
`FEATURE SET {0001}` driving the rail (done, and that is what the rail's page list is) and DPI as the
first new page (still a stub in both worlds — it was never built in Slint either).
