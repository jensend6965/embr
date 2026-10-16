#!/usr/bin/env python3
"""Dump the tray pixmap the app publishes for each icon style, as a picture.

The pixmap is exactly what a panel would draw, so this is evidence about the icon itself rather
than about a screenshot of the bar. Each style gets its own run of the app, against a throwaway
XDG_CONFIG_HOME, and the run ends the moment the pixmap has been read.
"""

import base64
import json
import os
import pathlib
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
BIN = HERE / ".." / ".." / "target" / "release" / "embr"
OUT = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/embr-tray-styles")
OUT.mkdir(parents=True, exist_ok=True)

# `ui` is written literally into the sandbox config. The ones marked `draw` become the picture; the
# rest are cases that should end up drawing something already in it (migration, unknown name).
CASES = [
    ("battery", "[ui]\ntray_style = \"battery\"\n", True),
    ("battery-vertical", "[ui]\ntray_style = \"battery-vertical\"\n", True),
    ("percent", "[ui]\ntray_style = \"percent\"\n", True),
    ("percent-white", "[ui]\ntray_style = \"percent\"\ntray_text_color = \"white\"\n", True),
    ("percent-blue", "[ui]\ntray_style = \"percent\"\ntray_text_color = \"blue\"\n", True),
    ("ring", "[ui]\ntray_style = \"ring\"\n", True),
    ("status", "[ui]\ntray_style = \"status\"\n", True),
    ("legacy-switch-on", "[ui]\ntray_shows_percent = true\n", False),
    ("unknown-name", "[ui]\ntray_style = \"hologram\"\n", False),
    ("unknown-colour", "[ui]\ntray_style = \"percent\"\ntray_text_color = \"chartreuse\"\n", False),
]


def pixmap(pid: int) -> list:
    name = f"org.kde.StatusNotifierItem-{pid}-1"
    for _ in range(60):
        done = subprocess.run(
            ["busctl", "--user", "--json=short", "get-property", name, "/StatusNotifierItem",
             "org.kde.StatusNotifierItem", "IconPixmap"],
            capture_output=True, text=True,
        )
        if done.returncode == 0 and done.stdout.strip():
            try:
                parsed = json.loads(done.stdout)
            except json.JSONDecodeError:
                time.sleep(0.5)
                continue
            icons = parsed.get("data") or []
            if icons and icons[0][0] > 0:
                return icons[0]
        time.sleep(0.5)
    raise SystemExit(f"no pixmap was published under {name}")


def as_bytes(raw) -> bytes:
    if isinstance(raw, str):
        return base64.b64decode(raw)
    return bytes(raw)


def write_ppm(icon: list, path: pathlib.Path) -> tuple[int, int]:
    width, height, raw = icon
    data = as_bytes(raw)
    pixels = bytearray()
    for index in range(width * height):
        # ARGB in network byte order.
        alpha, r, g, b = data[index * 4:index * 4 + 4]
        # The picture is drawn on the bar's own colour, so a partly transparent pixel is shown as
        # it would appear there rather than as a black fringe.
        background = (0x2E, 0x30, 0x36)
        mix = lambda channel, back: (channel * alpha + back * (255 - alpha)) // 255
        pixels += bytes((mix(r, background[0]), mix(g, background[1]), mix(b, background[2])))
    with path.open("wb") as handle:
        handle.write(f"P6\n{width} {height}\n255\n".encode())
        handle.write(bytes(pixels))
    return width, height


tiles = []
for name, config, draw in CASES:
    home = OUT / name
    (home / "embr").mkdir(parents=True, exist_ok=True)
    (home / "embr" / "config.toml").write_text(config)
    log = (OUT / f"{name}.log").open("wb")
    app = subprocess.Popen([BIN], env={**os.environ, "XDG_CONFIG_HOME": str(home)},
                           stdout=log, stderr=subprocess.STDOUT)
    try:
        # Wait for a sweep with something in it: the icon is only drawn with a level once the
        # devices have answered, and before that every style legitimately looks the same.
        read = False
        for _ in range(120):
            text = (OUT / f"{name}.log").read_text(errors="replace") if (OUT / f"{name}.log").exists() else ""
            if "device(s) read" in text:
                read = True
                break
            time.sleep(0.5)
        icon = pixmap(app.pid)
        ppm = OUT / f"{name}.ppm"
        width, height = write_ppm(icon, ppm)
        print(f"{name:18} {width}x{height}  logged a sweep: {read}  drew: {draw}")
        if draw:
            tiles.append((name, ppm))
    finally:
        app.terminate()
        try:
            app.wait(timeout=10)
        except subprocess.TimeoutExpired:
            app.kill()
        log.close()

# One strip: the icons at 10x with the style name above each, on the bar's own grey. The label is
# spliced on above the icon rather than annotated over it — the point is to see the shape.
for name, ppm in tiles:
    subprocess.run(["magick", str(ppm), "-filter", "point", "-resize", "1000%",
                    "-background", "#2E3036", "-gravity", "north", "-splice", "0x44",
                    "-fill", "#c8ccd2", "-pointsize", "24", "-annotate", "+0+8", name,
                    "-background", "#2E3036", "-gravity", "center", "-extent", "250x290",
                    str(OUT / f"{name}.png")], check=True)
subprocess.run(["magick", *[str(OUT / f"{name}.png") for name, _ in tiles],
                "+append", str(OUT / "styles.png")], check=True)
print("wrote", OUT / "styles.png")
