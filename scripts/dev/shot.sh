#!/usr/bin/env bash
# Screenshot the app's own window, against a throwaway configuration.
#
# The real ~/.config/embr/config.toml is never touched: XDG_CONFIG_HOME points at a sandbox, and the
# window is raised by the KWin script next to this one so `spectacle -a` captures the app rather
# than whatever else happens to be focused.
#
#   ./shot.sh <sandbox-config-dir> <out.png> [settings|devices] [scroll-pixels]
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="$HERE/../../target/release/embr"
SANDBOX="${1:?a sandbox config dir is needed}"
OUT="${2:?an output png is needed}"
PAGE="${3:-settings}"
SCROLL="${4:-0}"

mkdir -p "$SANDBOX/embr"
[[ -f "$SANDBOX/embr/config.toml" ]] || : > "$SANDBOX/embr/config.toml"

PAGE_FLAG=0
[[ "$PAGE" == settings ]] && PAGE_FLAG=1

# The debug switch is read with `is_some()`, so it must be *absent* for the other pages, not "0".
EXTRA=()
[[ "$PAGE_FLAG" == 1 ]] && EXTRA=(EMBR_DEBUG_SETTINGS=1)

env XDG_CONFIG_HOME="$SANDBOX" EMBR_DEBUG_SCROLL="$SCROLL" "${EXTRA[@]}" \
    "$BIN" > "$SANDBOX/app.log" 2>&1 &
APP=$!
trap 'kill "$APP" 2>/dev/null || true' EXIT

# Wait for a sweep to have been logged: the window is up by then, and the devices page has content.
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
