#!/usr/bin/env bash
# Screenshot the app's own window, against a throwaway configuration.
#
# The real ~/.config/embr/config.toml is never touched: XDG_CONFIG_HOME points at a sandbox, and the
# window is raised by the KWin script next to this one so `spectacle -a` captures the app rather
# than whatever else happens to be focused.
#
#   ./shot.sh <sandbox-config-dir> <out.png> [devices|settings|<page id>]
#
# `devices` is the window as it opens. `settings` opens the settings dialog at startup. Anything
# else is a page id (`battery`, `dpi`, `lighting`, `buttons`, `profiles`, `host`, `device`) and
# opens the window on that page of the first device — it waits for the device's feature set, and a
# page the device does not have will not be shown.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="$HERE/../../target/release/embr"
SANDBOX="${1:?a sandbox config dir is needed}"
OUT="${2:?an output png is needed}"
PAGE="${3:-devices}"

mkdir -p "$SANDBOX/embr"
[[ -f "$SANDBOX/embr/config.toml" ]] || : > "$SANDBOX/embr/config.toml"

# Each switch is read with `is_some()`, so it must be *absent* when it is not wanted, not "0".
EXTRA=()
case "$PAGE" in
    settings) EXTRA=(EMBR_DEBUG_SETTINGS=1) ;;
    devices) ;;
    *) EXTRA=("EMBR_DEBUG_PAGE=$PAGE") ;;
esac

env XDG_CONFIG_HOME="$SANDBOX" "${EXTRA[@]}" "$BIN" > "$SANDBOX/app.log" 2>&1 &
APP=$!
trap 'kill "$APP" 2>/dev/null || true' EXIT

# Wait for a sweep to have been logged: the window is up by then, and the rail has content.
for _ in $(seq 1 90); do
    grep -q "device(s) read" "$SANDBOX/app.log" 2>/dev/null && break
    sleep 0.5
done
sleep 2

# Raise and focus the app's window through KWin, then capture just that window. The script is
# loaded under a fresh name each time: KWin keeps the previous run loaded, and reloading the same
# name does not re-run it against a new window.
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript \
    "$HERE/kwin-raise-windows.js" "embr-raise-$$" >/dev/null
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.start >/dev/null
sleep 2

spectacle -b -n -a -o "$OUT"
sleep 1
magick identify "$OUT"
