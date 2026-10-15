#!/usr/bin/env bash
# Build and run spike 001. Keeps CARGO_TARGET_DIR/local paths deterministic.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
unset CARGO_TARGET_DIR
cd "$(dirname "$0")"
cargo build --target-dir target
exec ./target/debug/battery "$@"
