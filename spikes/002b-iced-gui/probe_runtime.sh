#!/usr/bin/env bash
# Spike 002b (iced) runtime probe: startup-to-window, VmRSS samples, screenshot.
# Detects the window via KWin's org.kde.KWin.queryWindowInfo() DBus call.
set -u
cd "$(dirname "$0")"
APP=./target/release/solaar-iced-spike
OUT=${OUT:-screenshot.png}
GLYPHS=${SPIKE_GLYPHS:-letters}

export SPIKE_GLYPHS="$GLYPHS"

T0=$(date +%s%N)
"$APP" > run.log 2>&1 &
PID=$!

DETECTED="no"
T1=""
for _ in $(seq 1 400); do
    if ! kill -0 "$PID" 2>/dev/null; then
        echo "!! process exited early"; cat run.log; exit 1
    fi
    if qdbus6 org.kde.KWin /KWin queryWindowInfo 2>/dev/null | grep -q 'caption: Devices'; then
        T1=$(date +%s%N)
        DETECTED="yes"
        break
    fi
    sleep 0.05
done
T1=${T1:-$(date +%s%N)}
echo "pid=$PID  window_detected=$DETECTED  startup_ms=$(( (T1 - T0) / 1000000 ))"

# Which display backend did winit/wgpu actually connect to?
echo -n "wayland_socket_fd="
ls -l /proc/$PID/fd 2>/dev/null | grep -c wayland || true
echo -n "x11_socket_fd="
ls -l /proc/$PID/fd 2>/dev/null | grep -cE "X11-unix|\.X11" || true
echo "--- run.log (stderr/stdout) ---"
cat run.log 2>/dev/null | head -20
echo "--- end run.log ---"

sleep 3
echo "VmRSS sample 1: $(grep VmRSS /proc/$PID/status)"
sleep 5
echo "VmRSS sample 2: $(grep VmRSS /proc/$PID/status)"
echo "VmHWM:          $(grep VmHWM /proc/$PID/status)"
echo "Threads:        $(grep Threads /proc/$PID/status)"
echo "GPU/vulkan fds: $(ls -l /proc/$PID/fd 2>/dev/null | grep -ciE 'renderD|dri|vulkan')"

sleep 1
spectacle -b -n -o "$OUT" 2>&1 | tail -2
ls -l "$OUT"
file "$OUT"

kill "$PID" 2>/dev/null
sleep 1
kill -9 "$PID" 2>/dev/null
echo "done"
