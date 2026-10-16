#!/usr/bin/env bash
# Build the GTK 4 prototype and run it.
#
#   ./run.sh                      # dark, as the desktop asks for
#   EMBR_SPIKE_COLOUR=1 ./run.sh  # … and open the stock colour picker at t+4s
#   EMBR_SPIKE_NO_FLIP=1 ./run.sh # don't flip the scheme at t+6s
#
# The prototype is a spike: it draws a hardcoded devices page. It reads no devices, writes no config,
# and registers no tray item.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
cd "$HERE"
cargo build --release
exec "./target/release/gtk-spike"
