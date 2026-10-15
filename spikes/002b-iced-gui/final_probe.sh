#!/usr/bin/env bash
# Final measurements for spike 002b against the *correct* binary.
# NOTE: this session has CARGO_TARGET_DIR pointing at a sibling spike's target
# dir, so the freshly built binary is taken from there and installed locally.
set -u
cd "$(dirname "$0")"
SHARED=${CARGO_TARGET_DIR:-}/release/solaar-iced-spike
BIN=target/release/solaar-iced-spike

echo "### binary provenance"
echo "shared build: $SHARED"
sha256sum "$SHARED" 2>/dev/null
mkdir -p target/release
cp "$SHARED" "$BIN"
sha256sum "$BIN"
echo -n "src/main.rs mtime: "; stat -c '%y' src/main.rs

echo "### size before strip"
stat -c 'bytes=%s' "$BIN"
file "$BIN"

echo "### strip"
strip -o /tmp/spike-stripped "$BIN"
stat -c 'stripped_bytes=%s' /tmp/spike-stripped
cp /tmp/spike-stripped "$BIN"
stat -c 'installed_bytes=%s' "$BIN"

echo "### ldd (stripped binary)"
ldd "$BIN"
objdump -p "$BIN" | grep -E "NEEDED|RPATH|RUNPATH"
echo -n "libs outside /usr/lib+/lib: "; ldd "$BIN" | awk '{print $3}' | grep '^/' | grep -vE '^/usr/lib|^/lib' || echo "(none)"

echo
echo "### startup timeline (native Wayland, protocol trace)"
T0=$(date +%s%N)
( WAYLAND_DEBUG=1 "$BIN" 2>&1 ) | python3 wayland_timeline.py "$T0" > wayland-startup.log 2>&1 &
sleep 10
pkill -f "$BIN" 2>/dev/null
pkill -f "solaar-iced-spike" 2>/dev/null
sleep 1
cat wayland-startup.log

echo
echo "### clean run: memory + window captures"
"$BIN" > run.log 2>&1 &
pid=$!
sleep 2
sleep 1
echo "VmRSS sample 1: $(grep VmRSS /proc/$pid/status)"
echo "Threads:        $(grep Threads /proc/$pid/status)"
sleep 5
echo "VmRSS sample 2: $(grep VmRSS /proc/$pid/status)"
echo "VmHWM:          $(grep VmHWM /proc/$pid/status)"

spectacle -b -n -o screenshot-fullscreen-wayland.png >/dev/null 2>&1
sleep 1
spectacle -b -n -a -o /tmp/win-wayland.png >/dev/null 2>&1
echo -n "wayland active-window capture: "; [ -s /tmp/win-wayland.png ] && identify -format "%wx%h %b\n" /tmp/win-wayland.png || echo "FAILED (empty)"
kill "$pid" 2>/dev/null; sleep 1; kill -9 "$pid" 2>/dev/null
echo "app log:"; cat run.log

echo "### XWayland run for the window-only capture"
env WAYLAND_DISPLAY= DISPLAY=:0 "$BIN" > run-x11.log 2>&1 &
pid=$!
sleep 3
spectacle -b -n -a -o /tmp/win-x11.png >/dev/null 2>&1
[ -s /tmp/win-x11.png ] && identify -format "x11 active-window capture: %wx%h %b\n" /tmp/win-x11.png || echo "x11 capture FAILED"
kill "$pid" 2>/dev/null; sleep 1; kill -9 "$pid" 2>/dev/null
echo "x11 app log:"; cat run-x11.log

echo "### choose the screenshot"
if [ -s /tmp/win-wayland.png ]; then
    cp /tmp/win-wayland.png screenshot.png ; echo "using native-Wayland window capture"
elif [ -s /tmp/win-x11.png ]; then
    cp /tmp/win-x11.png screenshot.png ; echo "using XWayland window capture"
fi
[ -f screenshot.png ] && identify -format "screenshot.png = %wx%h bytes=%b\n" screenshot.png
stat -c 'screenshot.png bytes=%s' screenshot.png
pkill -f solaar-iced-spike 2>/dev/null
echo "### done"
