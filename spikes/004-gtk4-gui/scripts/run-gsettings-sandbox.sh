#!/usr/bin/env bash
# Isolate the source of the colour scheme: run the spike against a *private* GSettings backend, so
# `gsettings set` here changes only the keyfile in the sandbox — the real desktop is untouched.
# If the window flips when the sandbox key flips, libadwaita reads GSettings live.
set -uo pipefail
S="$(cd "$(dirname "$0")/.." && pwd)"   # the spike directory
SB=$S/gs-sandbox
BIN=$S/target/release/gtk-spike
LOG=$S/run.log
RAISE=$S/../../scripts/dev/kwin-raise-windows.js

rm -rf "$SB"; mkdir -p "$SB"

mean() { magick "$1" -crop 1240x1000+1315+50 +repage -format '%[fx:mean]' info: 2>/dev/null; }

sandbox_gsettings() { env XDG_CONFIG_HOME="$SB" GSETTINGS_BACKEND=keyfile gsettings "$@"; }

echo "sandbox sees: $(sandbox_gsettings get org.gnome.desktop.interface color-scheme)"
echo "the desktop sees: $(gsettings get org.gnome.desktop.interface color-scheme)"

rm -f "$LOG"
env XDG_CONFIG_HOME="$SB" GSETTINGS_BACKEND=keyfile EMBR_SPIKE_NO_FLIP=1 "$BIN" > "$LOG" 2>&1 &
APP=$!
sleep 1
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript "$RAISE" "gtk-spike-$RANDOM" >/dev/null
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.start >/dev/null
sleep 2

spectacle -b -n -o "$S/gs-before.png" >/dev/null 2>&1
echo "before (sandbox key 'default'): mean=$(mean $S/gs-before.png)  $(grep -m1 'style manager' $LOG | sed 's/\[spike\] //')"

sandbox_gsettings set org.gnome.desktop.interface color-scheme prefer-light
sleep 3
spectacle -b -n -o "$S/gs-after.png" >/dev/null 2>&1
echo "after  (sandbox key 'prefer-light'): mean=$(mean $S/gs-after.png)"

echo "--- app log: did anything notice? ---"
grep -E "t\+[0-9]+s|scheme changed|style manager" "$LOG" | tail -6

kill $APP 2>/dev/null; wait $APP 2>/dev/null
echo "--- the real desktop's setting is still: $(gsettings get org.gnome.desktop.interface color-scheme) ---"
