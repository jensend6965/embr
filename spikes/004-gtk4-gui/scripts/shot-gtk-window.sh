#!/usr/bin/env bash
# One clean window-only photograph of the GTK spike, for the side-by-side.
set -uo pipefail
S="$(cd "$(dirname "$0")/.." && pwd)"   # the spike directory
BIN=$S/target/release/gtk-spike
LOG=$S/run.log
RAISE=$S/../../scripts/dev/kwin-raise-windows.js
OUT=${1:-$S/gtk-window.png}

rm -f "$LOG"
env EMBR_SPIKE_NO_FLIP=1 "$BIN" > "$LOG" 2>&1 &
APP=$!
sleep 1
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript "$RAISE" "gtk-shot-$RANDOM" >/dev/null
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.start >/dev/null
sleep 3
spectacle -b -n -o "$OUT"
sleep 0.5
printf 'shot: %s  %s\n' "$(magick identify -format '%wx%h' "$OUT")" "$OUT"
grep -E "style manager|accent|presented" "$LOG"
kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null
