#!/usr/bin/env bash
set -u
export WAYLAND_DISPLAY=wayland-0
cd <repo>/spikes/002c-egui-gui
LOG=/tmp/egui-shot4.log; : > "$LOG"
rm -f c_*.png
./target/release/spike-egui > "$LOG" 2>&1 &
PID=$!
for _ in $(seq 1 100); do grep -q SPIKE_FIRST_FRAME_MS "$LOG" 2>/dev/null && break; sleep 0.05; done
echo "pid=$PID $(cat "$LOG")"
for i in 1 2 3 4 5 6 7 8; do
  sleep 3
  if kill -0 "$PID" 2>/dev/null; then
     rss=$(awk '/VmRSS/{print $2}' /proc/$PID/status)
     spectacle -b -n -o "$PWD/c_$i.png" >/dev/null 2>&1
     echo "t=$((i*3))s alive rss=${rss}kB shot=c_$i.png"
  else
     echo "t=$((i*3))s PROCESS DIED"; break
  fi
done
kill "$PID" 2>/dev/null; sleep 1
echo "--- stderr log ---"; cat "$LOG"
ls -l c_*.png 2>/dev/null | wc -l
