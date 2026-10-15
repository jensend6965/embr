#!/usr/bin/env bash
# Spike 002b (iced): final runtime probe.
#  * startup-to-window on a clean (untraced) run, detected by polling the screen
#  * two VmRSS samples 5s apart
#  * the deliverable screenshot
set -u
cd "$(dirname "$0")"
APP=./target/release/solaar-iced-spike
OUT=${OUT:-screenshot.png}
TMP=$(mktemp -d)
export SPIKE_GLYPHS=${SPIKE_GLYPHS:-letters}

echo "== baseline capture =="
spectacle -b -n -o "$TMP/base.png" >/dev/null 2>&1
ls -l "$TMP/base.png" | awk '{print "baseline bytes:", $5}'

echo "== launching app (clean run) =="
T0=$(date +%s%N)
"$APP" > run.log 2>&1 &
pid=$!

DETECTED_MS=""
i=0
while [ $i -lt 40 ]; do
    i=$((i + 1))
    sleep 0.15
    spectacle -b -n -o "$TMP/poll.png" >/dev/null 2>&1
    DIFF=$(magick compare -metric RMSE "$TMP/base.png" "$TMP/poll.png" null: 2>&1 | grep -oE '\(0\.[0-9]+\)' | tr -d '()')
    [ -z "$DIFF" ] && DIFF=0
    # window is up when the screen differs clearly from the baseline
    if awk "BEGIN{exit !($DIFF > 0.02)}"; then
        T1=$(date +%s%N)
        DETECTED_MS=$(( (T1 - T0) / 1000000 ))
        echo "screen changed on poll #$i (RMSE=$DIFF) -> startup_ms=$DETECTED_MS (poll granularity ~spectacle cost)"
        break
    fi
done
[ -z "$DETECTED_MS" ] && echo "!! window never detected via screen diff"

echo "== idle 3s =="
sleep 3
echo "VmRSS sample 1: $(grep VmRSS /proc/$pid/status)"
echo "Threads:        $(grep Threads /proc/$pid/status)"
sleep 5
echo "VmRSS sample 2: $(grep VmRSS /proc/$pid/status)"
echo "VmHWM:          $(grep VmHWM /proc/$pid/status)"

echo "== screenshot =="
spectacle -b -n -o "$OUT" >/dev/null 2>&1
sleep 1
spectacle -b -n -o "$OUT" >/dev/null 2>&1
ls -l "$OUT"
file "$OUT"
echo "png bytes: $(stat -c %s "$OUT")"

echo "== run.log =="
cat run.log 2>/dev/null | head -10

kill "$pid" 2>/dev/null
sleep 1
kill -9 "$pid" 2>/dev/null
rm -rf "$TMP"
echo "== done =="
