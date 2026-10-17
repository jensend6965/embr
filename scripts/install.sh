#!/usr/bin/env bash
# Install Embr for one user: build it, put it under ~/.local, and give the launcher an entry.
#
#   ./scripts/install.sh              # build + install; Embr appears in your launcher
#   ./scripts/install.sh --uninstall   # take it back out
#
# No root, no package manager, no flags, and nothing that stays running: the three files below are
# the ones a desktop already looks at. Re-running it is how you update — it rebuilds and copies the
# new binary over the old one. A copy that is already running keeps the build it started with, so
# quit it from the tray and start it again afterwards.
set -euo pipefail

uninstall=false
case "${1:-}" in
    "") ;;
    --uninstall) uninstall=true ;;
    -h | --help)
        awk 'NR>1 && /^#/ { sub(/^# ?/, ""); print; next } NR>1 { exit }' "$0"
        exit 0
        ;;
    *)
        echo "install.sh: unknown argument '$1' (try --help)" >&2
        exit 2
        ;;
esac

# A rustup toolchain lives here and is often not on PATH in a fresh shell.
PATH="$HOME/.cargo/bin:$PATH"

# Where a desktop looks for a user-installed application, following the XDG variables when they are
# set: the binary on PATH, the icon and the launcher entry in the data dir.
bin_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
entry="$data_dir/applications/io.github.jensend6965.Embr.desktop"
icon="$data_dir/icons/hicolor/scalable/apps/embr.svg"

refresh_caches() {
    # Both tools are optional: without them the desktop scans the directories itself, so a missing
    # one is not an error — only an old cache hiding a new icon would be, and that is what -f fixes.
    command -v gtk-update-icon-cache >/dev/null &&
        gtk-update-icon-cache -f -t "$data_dir/icons/hicolor" >/dev/null 2>&1 || true
    command -v update-desktop-database >/dev/null &&
        update-desktop-database "$data_dir/applications" >/dev/null 2>&1 || true
}

if $uninstall; then
    rm -f "$bin_dir/embr" "$entry" "$icon"
    refresh_caches
    echo "Embr uninstalled."
    echo "  settings kept in ${XDG_CONFIG_HOME:-$HOME/.config}/embr — delete that directory to forget them."
    exit 0
fi

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"

# The libraries the build reads. Saying so here beats a system-deps failure twenty lines into cargo.
missing=""
command -v cargo >/dev/null || missing="$missing cargo (rustup.rs)"
command -v pkg-config >/dev/null || missing="$missing pkg-config"
if command -v pkg-config >/dev/null; then
    pkg-config --exists gtk4 || missing="$missing gtk4 (4.12 or newer, with headers)"
    pkg-config --exists libadwaita-1 || missing="$missing libadwaita-1 (1.5 or newer, with headers)"
    pkg-config --exists dbus-1 || missing="$missing dbus-1 (with headers)"
fi
if [[ -n "$missing" ]]; then
    echo "install.sh: cannot build yet, missing:$missing" >&2
    echo "  the package names per distribution are in README.md, under 'Build from source'." >&2
    exit 1
fi

cargo build --release -p embr

install -Dm755 target/release/embr "$bin_dir/embr"
install -Dm644 assets/embr.svg "$icon"
# The tracked entry says `Exec=embr`, which is what a packaged copy wants — a package puts the
# binary in /usr/bin, which is always on PATH. A user-local install cannot count on ~/.local/bin
# being on PATH in every session (a desktop started from a display manager does not always read the
# shell profile), and a launcher entry that resolves to nothing does nothing when clicked. So this
# copy names the binary by its full path; the asset itself stays the package-friendly one.
mkdir -p "$(dirname "$entry")"
sed "s|^Exec=embr\$|Exec=$bin_dir/embr|" assets/io.github.jensend6965.Embr.desktop >"$entry"
chmod 644 "$entry"
refresh_caches

if command -v desktop-file-validate >/dev/null; then
    desktop-file-validate "$entry" ||
        echo "install.sh: the desktop entry above is not valid; the launcher may show nothing" >&2
fi

echo "Embr installed."
echo "  $bin_dir/embr"
echo "  $entry"
echo "  $icon"
echo "Start it from your launcher, or run: embr"
