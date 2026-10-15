#!/usr/bin/env bash
# Final build measurements for spike 002b: cold release build, binary size, ldd.
set -u
cd "$(dirname "$0")"
BIN=target/release/solaar-iced-spike
LOG=build-release.log

echo "### cargo clean"
cargo clean

echo "### cold release build (timed)"
( time cargo build --release ) > "$LOG" 2>&1
tail -3 "$LOG"
grep -E "^real|^user|^sys" "$LOG"

echo "### binary size"
stat -c 'unstripped_bytes=%s' "$BIN"
strip -o /tmp/spike-stripped "$BIN"
stat -c 'stripped_bytes=%s' /tmp/spike-stripped
cp /tmp/spike-stripped "$BIN"
stat -c 'final_binary_bytes=%s' "$BIN"
du -h "$BIN"

echo "### ldd"
ldd "$BIN"
echo "### DT_NEEDED / RUNPATH"
objdump -p "$BIN" | grep -E "NEEDED|RPATH|RUNPATH"
echo "### non-standard linked libs (outside /usr/lib, /lib, ld-linux):"
ldd "$BIN" | awk '{print $3}' | grep '^/' | grep -vE '^/usr/lib|^/lib' || echo "(none)"
