#!/usr/bin/env bash
# Spike 002b (iced): Wayland protocol-timeline startup measurement + backend check.
set -u
cd "$(dirname "$0")"

T0=$(date +%s%N)
# Run the app under a Wayland protocol trace; timestamps come from the reader.
( WAYLAND_DEBUG=1 ./target/release/solaar-iced-spike 2>&1 ) | python3 wayland_timeline.py "$T0" > wayland-startup.log 2>&1 &
READER=$!

APP=$(pgrep -f 'target/release/solaar-iced-spike' | head -1)
echo "app_pid=$APP  reader_pid=$READER"

sleep 4
echo "=== backend linkage (from /proc/PID/maps) ==="
if [ -n "$APP" ] && [ -r "/proc/$APP/maps" ]; then
    echo "libwayland-client mappings: $(grep -c 'libwayland-client' /proc/$APP/maps || true)"
    echo "libX11 mappings:            $(grep -c 'libX11' /proc/$APP/maps || true)"
    echo "libxcb mappings:            $(grep -c 'libxcb' /proc/$APP/maps || true)"
    echo "libvulkan mappings:         $(grep -c 'libvulkan' /proc/$APP/maps || true)"
    echo "libEGL mappings:            $(grep -c 'libEGL' /proc/$APP/maps || true)"
    echo "=== VmRSS after 4s ==="; grep -E 'VmRSS|Threads' /proc/$APP/status
    sleep 5
    echo "=== VmRSS after 9s ==="; grep -E 'VmRSS|Threads' /proc/$APP/status
else
    echo "!! app process not found"
fi

pkill -f 'target/release/solaar-iced-spike' 2>/dev/null
wait $READER 2>/dev/null
echo "=== timeline ==="
cat wayland-startup.log
