#!/bin/bash
# Spike 002a: cold-start wall time, process spawn -> window shown.
# Launcher stamps CLOCK_REALTIME ns immediately before spawn; the app prints
# WINDOW_SHOWN_MS (ms since the same epoch) right after ui.show() returns.
APP=<repo>/spikes/002a-slint-gui/target/release/devices
LOG=/tmp/devices_startup.log
: > "$LOG"

start=$(date +%s%N)
"$APP" > "$LOG" 2>&1 &
pid=$!

for i in $(seq 1 1000); do
    grep -q "WINDOW_SHOWN_MS=" "$LOG" 2>/dev/null && break
    sleep 0.01
done
end=$(date +%s%N)

shown=$(grep -o 'WINDOW_SHOWN_MS=[0-9]*' "$LOG" | cut -d= -f2)
echo "pid=$pid"
echo "launcher_ms_total=$(( (end - start) / 1000000 ))"
if [ -n "$shown" ]; then
    echo "startup_ms=$(( shown - start / 1000000 ))"
else
    echo "startup_ms=NONE (marker not found)"
fi
kill "$pid" 2>/dev/null
sleep 0.4
kill -9 "$pid" 2>/dev/null
