#!/usr/bin/env bash
# Probe whether Noctalia puts un-pinned SNI items in its tray drawer.
# Usage: ./probe_drawer.sh <tag> [extra env assignments...]
set -u
export PATH="$HOME/.cargo/bin:$PATH"
SPIKE=<repo>/spikes/003-tray-and-notifications
EV=$SPIKE/evidence
LOG=/home/jensend/.local/share/noctalia-kwe.log
TAG="${1:-drawer}"
shift || true

cd "$SPIKE"
cargo build --release 2>&1 | tail -2
MARK=$(wc -l < "$LOG")

env "$@" "$SPIKE/target/release/solaar-tray-spike" > "$EV/$TAG.log" 2>&1 &
PID=$!
echo "spike pid=$PID" | tee "$EV/$TAG.meta"
sleep 3

shoot() { timeout 30 spectacle -b -n -o "$EV/$TAG-$1.png" >/dev/null 2>&1; }

shoot bar          # tray item up, nothing else
for pid_cand in tray-drawer tray tray_drawer; do
  out=$(/usr/bin/noctalia-kwe msg panel-open "$pid_cand" 2>&1)
  echo "panel-open $pid_cand -> $out" | tee -a "$EV/$TAG.meta"
  sleep 2
  shoot "panel-$pid_cand"
done

# also try the tray-drawer launcher provider and control-center
for cmd in "panel-open control-center" "panel-open notifications"; do
  out=$(/usr/bin/noctalia-kwe msg $cmd 2>&1)
  echo "msg $cmd -> $out" | tee -a "$EV/$TAG.meta"
  sleep 1
done
shoot control-center
echo "panelOpen state: $(/usr/bin/noctalia-kwe msg status 2>&1 | tr -d '\n')" | tee -a "$EV/$TAG.meta"
/usr/bin/noctalia-kwe msg panel-close 2>&1 | tee -a "$EV/$TAG.meta"

wait $PID
echo "--- noctalia log since mark ---" | tee -a "$EV/$TAG.meta"
tail -n +$((MARK+1)) "$LOG" | grep -i -E "tray|icon" | tee -a "$EV/$TAG.meta"
ls -l "$EV"/$TAG-*.png | tee -a "$EV/$TAG.meta"
