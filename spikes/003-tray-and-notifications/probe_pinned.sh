#!/usr/bin/env bash
# Test: does Noctalia's bar tray show only items whose SNI Id is in [widget.tray].pinned ?
# Usage: ./probe_pinned.sh <tag> [extra env assignments...]
set -u
export PATH="$HOME/.cargo/bin:$PATH"
SPIKE=/home/jensend/Projects/logi-modern/spikes/003-tray-and-notifications
EV=$SPIKE/evidence
LOG=/home/jensend/.local/share/noctalia-kwe.log
TAG="${1:-pinned}"
shift || true

cd "$SPIKE"
cargo build --release 2>&1 | tail -2
MARK=$(wc -l < "$LOG")

env "$@" "$SPIKE/target/release/solaar-tray-spike" > "$EV/$TAG.log" 2>&1 &
PID=$!
echo "spike pid=$PID" | tee "$EV/$TAG.meta"
sleep 3

timeout 30 spectacle -b -n -o "$EV/$TAG-bar.png" >/dev/null 2>&1

# Arm a delayed screenshot, THEN open the drawer so the drawer is still open when it fires.
( timeout 30 spectacle -b -n -d 1 -o "$EV/$TAG-drawer.png" >/dev/null 2>&1 ) &
sleep 0.3
/usr/bin/noctalia-kwe msg panel-open tray-drawer 2>&1 | tee -a "$EV/$TAG.meta"
wait $PID
echo "--- noctalia log since mark ---" | tee -a "$EV/$TAG.meta"
tail -n +$((MARK+1)) "$LOG" | grep -i -E "tray|panel" | tee -a "$EV/$TAG.meta"
ls -l "$EV/$TAG-bar.png" "$EV/$TAG-drawer.png" | tee -a "$EV/$TAG.meta"
