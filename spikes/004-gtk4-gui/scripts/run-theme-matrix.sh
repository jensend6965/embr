#!/usr/bin/env bash
# Which route actually restyles the window? Four short runs, window-only captures, mean brightness.
#   A  as it opens (the desktop says dark)          -> expect dark
#   B  GTK_THEME=Adwaita:light from outside         -> expect light
#   C  libadwaita set_color_scheme(ForceLight) at startup
#   D  Gtk.Settings gtk-theme-name=Adwaita:light at startup
#   E  libadwaita set_color_scheme at t+4s (runtime)
#   F  Gtk.Settings gtk-theme-name at t+4s (runtime)
set -uo pipefail
S="$(cd "$(dirname "$0")/.." && pwd)"   # the spike directory
BIN=$S/target/release/gtk-spike
LOG=$S/run.log
RAISE=$S/../../scripts/dev/kwin-raise-windows.js

raise() {
    qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript "$RAISE" "gtk-spike-$RANDOM" >/dev/null
    qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.start >/dev/null
    sleep 2
}

run() { # run <label> <env...>
    local label="$1"; shift
    rm -f "$LOG"
    env EMBR_SPIKE_NO_FLIP=1 "$@" "$BIN" > "$LOG" 2>&1 &
    local app=$!
    sleep 1
    raise
    sleep 2
    spectacle -b -n -a -o "$S/v-$label.png" >/dev/null 2>&1
    local mean
    mean=$(magick "$S/v-$label.png" -format '%[fx:mean]' info: 2>/dev/null)
    local dims
    dims=$(magick identify -format '%wx%h' "$S/v-$label.png" 2>/dev/null)
    printf '%-12s mean=%-8s %s   %s\n' "$label" "$mean" "$dims" \
        "$(grep -m1 'accent:' "$LOG" | sed 's/\[spike\] //')"
    kill $app 2>/dev/null; wait $app 2>/dev/null
}

echo "A default (desktop dark)";                run a
echo "B GTK_THEME=Adwaita:light";               run b GTK_THEME=Adwaita:light
echo "C libadwaita at startup";                 run c EMBR_SPIKE_AT_STARTUP=libadwaita
echo "D gtk-theme-name at startup";             run d EMBR_SPIKE_AT_STARTUP=gtk-theme
echo "E libadwaita at t+4s";                    run e EMBR_SPIKE_AT_4S=libadwaita
echo "F gtk-theme-name at t+4s";                run f EMBR_SPIKE_AT_4S=gtk-theme
echo
echo "--- E's log tail (is_dark vs pixels) ---"
grep -E "t\+[0-9]+s|switching" "$LOG" | tail -4
