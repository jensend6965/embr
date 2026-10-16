#!/usr/bin/env bash
# Photograph the roadmap-controls page: the spike adds a profile row by itself at t+3s and shows a
# toast, so the screenshot can prove those behaviours without input injection.
set -uo pipefail
S="$(cd "$(dirname "$0")/.." && pwd)"   # the spike directory
BIN=$S/target/release/gtk-spike
LOG=$S/run.log
RAISE=$S/../../scripts/dev/kwin-raise-windows.js

rm -f "$LOG"
env EMBR_SPIKE_NO_FLIP=1 EMBR_SPIKE_COLOUR=1 "$BIN" > "$LOG" 2>&1 &
APP=$!
sleep 1
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript "$RAISE" "gtk-road-$RANDOM" >/dev/null
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.start >/dev/null
sleep 7
spectacle -b -n -o "$S/gtk-roadmap.png"; sleep 0.4
spectacle -b -n -a -o "$S/gtk-roadmap-window.png"; sleep 0.2
magick identify -format '%f %wx%h\n' "$S/gtk-roadmap.png" "$S/gtk-roadmap-window.png"
grep -E "accent|presented" "$LOG"
kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null
