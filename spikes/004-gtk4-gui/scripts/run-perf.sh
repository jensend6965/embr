#!/usr/bin/env bash
# Same machine, same minute: idle footprint of the Slint app (Embr today) and the GTK 4 prototype.
# Both are started against a throwaway config / no config, left alone for 20 s, then measured.
set -uo pipefail
S="$(cd "$(dirname "$0")/.." && pwd)"   # the spike directory
mkdir -p "$S/perf-sb/embr"
: > "$S/perf-sb/embr/config.toml"

measure() { # measure <label> <logfile> <pid>
    ps -o rss=,vsz=,time=,pcpu= -p "$3" | awk -v l="$1" '{ printf "%-10s RSS %6.1f MiB   VSZ %7.1f MiB   CPU time %-8s  %s%%\n", l, $1/1024, $2/1024, $3, $4 }'
    grep -m1 -E "window shown|presented after" "$2" | sed 's/^/           /'
}

XDG_CONFIG_HOME="$S/perf-sb" $S/../../target/release/embr > "$S/perf-slint.log" 2>&1 &
P1=$!
sleep 22
measure "Slint" "$S/perf-slint.log" "$P1"

EMBR_SPIKE_NO_FLIP=1 "$S/target/release/gtk-spike" > "$S/perf-gtk.log" 2>&1 &
P2=$!
sleep 22
measure "GTK4" "$S/perf-gtk.log" "$P2"

echo "--- threads / open fds ---"
printf 'Slint  threads=%s fds=%s\n' "$(ls /proc/$P1/task | wc -l)" "$(ls /proc/$P1/fd | wc -l)"
printf 'GTK4   threads=%s fds=%s\n' "$(ls /proc/$P2/task | wc -l)" "$(ls /proc/$P2/fd | wc -l)"

kill $P1 $P2 2>/dev/null
wait $P1 $P2 2>/dev/null
echo "--- both stopped ---"
