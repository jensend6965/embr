#!/usr/bin/env bash
# Assemble the final artifacts.
#  * screenshot.png  -> the verified window-only capture of the app (XWayland run;
#                       on this compositor `spectacle -a` grabs whatever window is
#                       focused, which is not always the spike window)
#  * screenshot-fullscreen-wayland.png -> fresh native-Wayland full-screen capture
set -u
cd "$(dirname "$0")"
BIN=target/release/solaar-iced-spike

echo "=== existing window capture ==="
identify -format "win-x11.png = %wx%h bytes=%b\n" win-x11.png
cp win-x11.png screenshot.png

echo "=== fresh native-Wayland full-screen capture ==="
"$BIN" > run.log 2>&1 &
pid=$!
sleep 4
spectacle -b -n -o screenshot-fullscreen-wayland.png >/dev/null 2>&1
sleep 0.5
spectacle -b -n -o screenshot-fullscreen-wayland.png >/dev/null 2>&1
identify -format "screenshot-fullscreen-wayland.png = %wx%h bytes=%b\n" screenshot-fullscreen-wayland.png
grep "window size" run.log | tail -1
kill "$pid" 2>/dev/null; sleep 1; kill -9 "$pid" 2>/dev/null

echo "=== unicode-glyph variant (documented limitation) ==="
mv -f glyphcheck-symbols.png screenshot-glyphs-unicode.png 2>/dev/null
identify -format "screenshot-glyphs-unicode.png = %wx%h\n" screenshot-glyphs-unicode.png 2>/dev/null

echo "=== tidy up scratch ==="
rm -f patch.png corner-compare.png candidates.png shot-tiled.png shot-symbols.png \
      run-*.log win-*.png shot-fixed.png probe_capture.sh probe_geometry.sh probe_variants.sh \
      probe_bbox.sh probe_activewindow.sh probe_capture_rect.sh probe_rect_shot.sh \
      probe_window_shot.sh find_window_origin.py kwin-info.txt
ls -la
