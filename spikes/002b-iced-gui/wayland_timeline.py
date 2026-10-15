#!/usr/bin/env python3
"""Timestamp a WAYLAND_DEBUG=1 stream to find when the first frame is presented.

Usage: WAYLAND_DEBUG=1 ./app 2>&1 | python3 wayland_timeline.py <t0_ns>
t0_ns is a CLOCK_REALTIME nanosecond stamp taken just before exec'ing the app.
"""
import sys
import time

t0 = int(sys.argv[1])
marks = {
    "first_wl_message": None,
    "create_surface": None,
    "xdg_surface_created": None,
    "first_configure": None,
    "first_attach": None,
    "first_commit": None,
    "first_damage": None,
    "first_scale": None,
}
head = []
nlines = 0

for raw in sys.stdin:
    nlines += 1
    line = raw.rstrip("\n")
    if len(head) < 40:
        head.append(line)
    now = time.time_ns()

    def mark(key):
        if marks[key] is None:
            marks[key] = now

    if marks["first_wl_message"] is None and line.startswith("["):
        mark("first_wl_message")
    if "xdg_wm_base@" in line and "get_xdg_surface" in line:
        mark("xdg_surface_created")
    if "xdg_toplevel@" in line and "commit" not in line and "set_" in line:
        mark("first_scale")  # first xdg_toplevel request (= window exists)
    if "configure(" in line and ("xdg_surface@" in line or "xdg_toplevel@" in line):
        mark("first_configure")
    if "attach(" in line:
        mark("first_attach")
    if "commit(" in line:
        mark("first_commit")
    if "damage(" in line or "damage_buffer(" in line:
        mark("first_damage")

print(f"# wayland protocol lines: {nlines}")
for key, value in marks.items():
    if value is None:
        print(f"{key:22s} never")
    else:
        print(f"{key:22s} {(value - t0) / 1e6:8.1f} ms after exec")

print("# first 20 protocol lines (evidence that Wayland was used):")
for line in head[:20]:
    print("#   " + line)

# A Wayland connection is proof the app used the native Wayland backend.
if marks["first_wl_message"] is not None:
    print("# BACKEND=wayland (native); no X11 fallback")
else:
    print("# BACKEND=unknown/X11 (no wayland protocol traffic)")
