#!/usr/bin/env bash
set -u
export WAYLAND_DISPLAY=wayland-0
cd <repo>/spikes/002c-egui-gui
./target/release/spike-egui > /tmp/egui-idle.log 2>&1 &
PID=$!
sleep 4
u1=$(awk '{print $14}' /proc/$PID/stat); s1=$(awk '{print $15}' /proc/$PID/stat)
HZ=$(getconf CLK_TCK)
sleep 5
u2=$(awk '{print $14}' /proc/$PID/stat); s2=$(awk '{print $15}' /proc/$PID/stat)
ticks=$(( (u2+s2)-(u1+s1) ))
echo "idle ticks over 5s: $ticks of $((HZ*5))  ->  $(python3 -c "print(f'{$ticks/$HZ/5*100:.2f}')")% CPU"
awk '/VmRSS/{print "VmRSS="$2" "$3}' /proc/$PID/status
awk '/^Threads/{print "Threads="$2}' /proc/$PID/status
kill $PID 2>/dev/null
sleep 1
echo "--- log ---"; cat /tmp/egui-idle.log
