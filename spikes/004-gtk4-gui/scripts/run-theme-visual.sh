#!/usr/bin/env bash
# Four runs, whole-desktop captures, so there is no ambiguity about what is in the picture:
#   a  as it opens (the desktop says dark)
#   b  GTK's own prefer-dark-theme off at startup
#   c  libadwaita set_color_scheme(ForceLight) at startup
#   d  libadwaita set_color_scheme(ForceLight) at t+4s, photographed at ~3s and ~7s (live switch)
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

shot() { spectacle -b -n -o "$S/t-$1.png" >/dev/null 2>&1; printf '   %s mean=%s\n' "$1" "$(magick $S/t-$1.png -crop 1290x990+1305+44 +repage -format '%[fx:mean]' info: 2>/dev/null)"; }

run() { # run <label> <env...>
    local label="$1"; shift
    rm -f "$LOG"
    env EMBR_SPIKE_NO_FLIP=1 "$@" "$BIN" > "$LOG" 2>&1 &
    local app=$!
    sleep 1; raise; sleep 1
    shot "$label"
    grep -m1 "accent:" "$LOG" | sed 's/^/   /'
    kill $app 2>/dev/null; wait $app 2>/dev/null
}

echo "a: as it opens";                     run a
echo "b: GTK prefer-dark off at startup"; run b EMBR_SPIKE_AT_STARTUP=gtk-theme
echo "c: libadwaita light at startup";    run c EMBR_SPIKE_AT_STARTUP=libadwaita

echo "d: libadwaita light at t+4s, two photographs"
rm -f "$LOG"
env EMBR_SPIKE_NO_FLIP=1 EMBR_SPIKE_AT_4S=libadwaita "$BIN" > "$LOG" 2>&1 &
APP=$!
sleep 1; raise
shot d-before
sleep 6
spectacle -b -n -o "$S/t-d-after.png" >/dev/null 2>&1
printf '   d-after mean=%s\n' "$(magick $S/t-d-after.png -crop 1290x990+1305+44 +repage -format '%[fx:mean]' info: 2>/dev/null)"
kill $APP 2>/dev/null; wait $APP 2>/dev/null
echo "--- log ---"; grep -E "t\+[0-9]+s|switching" "$LOG" | tail -5
