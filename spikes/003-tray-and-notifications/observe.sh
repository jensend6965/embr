#!/usr/bin/env bash
# Observation harness for spike 003.
# Usage: ./observe.sh <tag> [extra env assignments...]
# Starts the spike, screenshots while tray+OSD are up, records RSS and D-Bus state.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
SPIKE=<repo>/spikes/003-tray-and-notifications
EV=$SPIKE/evidence
TAG="${1:-run1}"
shift || true

cd "$SPIKE"
cargo build --release 2>&1 | tail -2

LOG=/home/jensend/.local/share/noctalia-kwe.log
MARK=$(wc -l < "$LOG")
echo "noctalia log mark line=$MARK" | tee "$EV/$TAG.meta"

rm -f "$EV/$TAG.log"
env "$@" "$SPIKE/target/release/solaar-tray-spike" > "$EV/$TAG.log" 2>&1 &
PID=$!
echo "spike pid=$PID" | tee -a "$EV/$TAG.meta"
sleep 3

echo "--- /proc/$PID/status (VmRSS, tray item up) ---" | tee -a "$EV/$TAG.meta"
grep -E 'VmRSS|VmSize|Threads' /proc/$PID/status 2>&1 | tee -a "$EV/$TAG.meta"

echo "--- own SNI bus name ---" | tee -a "$EV/$TAG.meta"
busctl --user list 2>&1 | grep -i "StatusNotifierItem-$PID" | tee -a "$EV/$TAG.meta"

echo "--- watcher RegisteredStatusNotifierItems ---" | tee -a "$EV/$TAG.meta"
gdbus call --session --dest org.kde.StatusNotifierWatcher \
  --object-path /StatusNotifierWatcher \
  --method org.kde.StatusNotifierWatcher.GetRegisteredItems 2>&1 | tee -a "$EV/$TAG.meta"

sleep 1
timeout 30 spectacle -b -n -o "$EV/$TAG-a.png" >/dev/null 2>&1
sleep 3
timeout 30 spectacle -b -n -o "$EV/$TAG-b.png" >/dev/null 2>&1
sleep 1
echo "--- screenshots ---" | tee -a "$EV/$TAG.meta"
ls -l "$EV/$TAG-a.png" "$EV/$TAG-b.png" 2>&1 | tee -a "$EV/$TAG.meta"
file "$EV/$TAG-a.png" "$EV/$TAG-b.png" | tee -a "$EV/$TAG.meta"

wait $PID
echo "--- spike exited ---" | tee -a "$EV/$TAG.meta"

echo "--- NOCTALIA LOG (tray/icon/statusnotifier lines since mark) ---" | tee -a "$EV/$TAG.meta"
tail -n +$((MARK+1)) "$LOG" | grep -i -E "tray|statusnotifier|icon" | tee -a "$EV/$TAG.meta"
