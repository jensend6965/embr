#!/usr/bin/env bash
# Spike 002c runtime measurement: startup, RSS, screenshot.
set -u
export PATH=$HOME/.cargo/bin:$PATH
cd /home/jensend/Projects/logi-modern/spikes/002c-egui-gui
export WAYLAND_DISPLAY=wayland-0

LOG=/tmp/egui-release.log
: > "$LOG"

echo "### launch -> first frame (external poll, date +%s%N)"
T0=$(date +%s%N)
./target/release/spike-egui > "$LOG" 2>&1 &
PID=$!
echo "pid=$PID"
for _ in $(seq 1 3000); do
  if grep -q SPIKE_FIRST_FRAME_MS "$LOG" 2>/dev/null; then break; fi
  sleep 0.02
done
T1=$(date +%s%N)
echo "launch_to_first_frame_log_ms=$(( (T1-T0)/1000000 ))"
echo -n "internal: "; grep SPIKE_FIRST_FRAME_MS "$LOG" || echo "(none)"
echo -n "window: "; grep SPIKE_WINDOW "$LOG" || echo "(inner_rect never reported on Wayland)"

echo
echo "### idle 3s, then RSS samples 5s apart"
sleep 3
R1=$(awk '/VmRSS/{print $2" "$3}' "/proc/$PID/status")
echo "VmRSS_sample1=$R1"
sleep 5
R2=$(awk '/VmRSS/{print $2" "$3}' "/proc/$PID/status")
echo "VmRSS_sample2=$R2"
echo "VmSize_now=$(awk '/VmSize/{print $2" "$3}' "/proc/$PID/status")"
echo "Threads=$(awk '/^Threads/{print $2}' "/proc/$PID/status")"
echo "fd_count=$(ls /proc/$PID/fd 2>/dev/null | wc -l)"
echo "open_mem_fds_for_gpu:"
ls -l /proc/$PID/fd 2>/dev/null | grep -oE '/dev/(dri|shm)[^ ]*|anon_inode:[^]]*]' | sort | uniq -c | head -10

echo
echo "### screenshot"
spectacle -b -n -o "$PWD/screenshot.png" 2>&1 | tail -2
sleep 1
ls -l screenshot.png
file screenshot.png

echo
echo "### kill"
kill "$PID" 2>/dev/null
sleep 1
pgrep -a spike-egui || echo "app stopped"
echo "### log tail"
cat "$LOG"
