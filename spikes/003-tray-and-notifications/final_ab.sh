#!/usr/bin/env bash
# Final A/B on the pinned allowlist + the money-shot capture (tray icon AND notification OSD together).
# Usage: ./final_ab.sh <tag> [extra env assignments...]
set -u
SPIKE=/home/jensend/Projects/logi-modern/spikes/003-tray-and-notifications
EV=$SPIKE/evidence
LOG=/home/jensend/.local/share/noctalia-kwe.log
TAG="${1:-d1}"
shift || true

/usr/bin/noctalia-kwe msg panel-close >/dev/null 2>&1
sleep 1
MARK=$(wc -l < "$LOG")

env "$@" "$SPIKE/target/release/solaar-tray-spike" > "$EV/$TAG.log" 2>&1 &
PID=$!
echo "spike pid=$PID tag=$TAG env: $*" | tee "$EV/$TAG.meta"
sleep 2
timeout 30 spectacle -b -n -m -o "$EV/$TAG-t2.png" >/dev/null 2>&1
sleep 2
timeout 30 spectacle -b -n -m -o "$EV/$TAG-t4.png" >/dev/null 2>&1
wait $PID
echo "--- noctalia log since mark ---" | tee -a "$EV/$TAG.meta"
tail -n +$((MARK+1)) "$LOG" | grep -i -E "tray|panel" | tee -a "$EV/$TAG.meta"
