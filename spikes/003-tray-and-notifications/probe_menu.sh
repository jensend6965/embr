#!/usr/bin/env bash
# Dump the live SNI + com.canonical.dbusmenu layout of the running tray item.
set -u
SPIKE=/home/jensend/Projects/logi-modern/spikes/003-tray-and-notifications
EV=$SPIKE/evidence
TAG="${1:-menu}"
shift || true

env "$@" "$SPIKE/target/release/solaar-tray-spike" > "$EV/$TAG.log" 2>&1 &
PID=$!
OWN="org.kde.StatusNotifierItem-$PID-1"
sleep 3
{
  echo "=== owner: $OWN ==="
  echo "--- busctl --user list (own name) ---"
  busctl --user list | grep -i "StatusNotifierItem-$PID"
  echo "--- watcher RegisteredStatusNotifierItems ---"
  gdbus call --session --dest org.kde.StatusNotifierWatcher --object-path /StatusNotifierWatcher \
    --method org.kde.StatusNotifierWatcher.GetRegisteredItems
  echo "--- /MenuBar introspection ---"
  gdbus introspect --session --dest "$OWN" --object-path /MenuBar 2>&1 | head -40
  echo "--- com.canonical.dbusmenu GetLayout ---"
  gdbus call --session --dest "$OWN" --object-path /MenuBar \
    --method com.canonical.dbusmenu.GetLayout 0 -- -1 "[]" 2>&1
  echo "--- com.canonical.dbusmenu GetGroupProperties(0, [label]) ---"
  gdbus call --session --dest "$OWN" --object-path /MenuBar \
    --method com.canonical.dbusmenu.GetGroupProperties 'uint32 0' "['label']" 2>&1
} | tee "$EV/$TAG.meta"
wait $PID
