#!/usr/bin/env bash
# Confirm the pinned-allowlist mechanism + capture the tray drawer + dump SNI/dbusmenu properties.
# Usage: ./probe_final.sh <tag> [extra env assignments...]
set -u
export PATH="$HOME/.cargo/bin:$PATH"
SPIKE=<repo>/spikes/003-tray-and-notifications
EV=$SPIKE/evidence
LOG=/home/jensend/.local/share/noctalia-kwe.log
TAG="${1:-final}"
shift || true

cd "$SPIKE"
cargo build --release 2>&1 | tail -2
MARK=$(wc -l < "$LOG")

env "$@" "$SPIKE/target/release/solaar-tray-spike" > "$EV/$TAG.log" 2>&1 &
PID=$!
OWN="org.kde.StatusNotifierItem-$PID-1"
echo "spike pid=$PID owner=$OWN" | tee "$EV/$TAG.meta"
sleep 3

timeout 30 spectacle -b -n -o "$EV/$TAG-bar.png" >/dev/null 2>&1

echo "--- SNI ToolTip property ---" | tee -a "$EV/$TAG.meta"
gdbus call --session --dest "$OWN" --object-path /StatusNotifierItem \
  --method org.freedesktop.DBus.Properties.Get org.kde.StatusNotifierItem ToolTip 2>&1 | tee -a "$EV/$TAG.meta"
echo "--- SNI Title/Id/IconName/IconPixmap size ---" | tee -a "$EV/$TAG.meta"
for p in Title Id Category Status IconName IconThemePath WindowId; do
  printf "  %-12s = " "$p" | tee -a "$EV/$TAG.meta"
  gdbus call --session --dest "$OWN" --object-path /StatusNotifierItem \
    --method org.freedesktop.DBus.Properties.Get org.kde.StatusNotifierItem "$p" 2>&1 | tee -a "$EV/$TAG.meta"
done
printf "  IconPixmap   = " | tee -a "$EV/$TAG.meta"
gdbus call --session --dest "$OWN" --object-path /StatusNotifierItem \
  --method org.freedesktop.DBus.Properties.Get org.kde.StatusNotifierItem IconPixmap 2>&1 \
  | sed 's/\(\[(\).\{0,120\}.*/\1<ARGB32 bytes elided>.../' | tee -a "$EV/$TAG.meta"

echo "--- dbusmenu object paths ---" | tee -a "$EV/$TAG.meta"
gdbus introspect --session --dest "$OWN" --object-path / 2>&1 | tee -a "$EV/$TAG.meta"
echo "--- dbusmenu GetLayout ---" | tee -a "$EV/$TAG.meta"
gdbus call --session --dest "$OWN" --object-path /MenuBar \
  --method com.canonical.dbusmenu.GetLayout 0 -1 '[]' 2>&1 | tee -a "$EV/$TAG.meta"

# Drawer capture: arm a delayed full-screen shot, THEN open the hover drawer.
( timeout 30 spectacle -b -n -m -d 1 -o "$EV/$TAG-drawer.png" >/dev/null 2>&1 ) &
sleep 0.3
/usr/bin/noctalia-kwe msg panel-open tray-drawer 2>&1 | tee -a "$EV/$TAG.meta"
sleep 2
echo "panel state: $(/usr/bin/noctalia-kwe msg status 2>&1 | tr -d '\n')" | tee -a "$EV/$TAG.meta"

wait $PID
echo "--- noctalia log since mark ---" | tee -a "$EV/$TAG.meta"
tail -n +$((MARK+1)) "$LOG" | grep -i -E "tray|panel" | tee -a "$EV/$TAG.meta"
ls -l "$EV/$TAG-bar.png" "$EV/$TAG-drawer.png" 2>&1 | tee -a "$EV/$TAG.meta"
