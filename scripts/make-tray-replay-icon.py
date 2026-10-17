#!/usr/bin/env python3
"""Generate `src-tauri/icons/tray-replay.rgba` — the tray icon while Replay is
on (v1.54.0).

It is the app's 32×32 icon with a filled red dot in the lower right, inside a
light ring so the dot reads on a dark taskbar as well as a light one. The tray
shows it for as long as Replay keeps the screen in memory, so a hidden Divora
still says so in the notification area (Windows' yellow border may be on a
monitor nobody is looking at).

The output is raw RGBA, 32 × 32 × 4 = 4096 bytes, rows top to bottom: what
`tauri::image::Image::new` takes, so the app needs no PNG decoder for it.

Python standard library only (the PNG is decoded with `zlib` here). Run from
anywhere; the result is deterministic, and is committed:

    python scripts/make-tray-replay-icon.py
"""

import struct
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "src-tauri" / "icons" / "32x32.png"
OUTPUT = ROOT / "src-tauri" / "icons" / "tray-replay.rgba"

SIZE = 32
# The dot: 12 px across, centred so its ring just meets the lower-right edge.
CENTRE = (24.5, 24.5)
DOT_RADIUS = 6.0
RING_RADIUS = 7.5  # a 1.5 px light ring around the dot
RED = (220, 38, 38)
LIGHT = (255, 255, 255)
# Sub-pixel samples per side, for smooth edges.
SAMPLES = 4


def read_png_rgba(path: Path) -> list[list[tuple[int, int, int, int]]]:
    """Decode an 8-bit, non-interlaced RGBA PNG into rows of (r, g, b, a)."""
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit(f"{path}: not a PNG")
    pos, idat, header = 8, b"", None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break
        pos += 12 + length
    if header is None:
        sys.exit(f"{path}: no IHDR")
    width, height, depth, colour, _, _, interlace = header
    if (depth, colour, interlace) != (8, 6, 0):
        sys.exit(f"{path}: expected 8-bit RGBA, not interlaced; got {header}")

    raw = zlib.decompress(idat)
    stride = width * 4
    rows, prev = [], bytes(stride)
    for y in range(height):
        start = y * (stride + 1)
        kind, line = raw[start], bytearray(raw[start + 1 : start + 1 + stride])
        for i in range(stride):
            left = line[i - 4] if i >= 4 else 0
            up = prev[i]
            up_left = prev[i - 4] if i >= 4 else 0
            if kind == 1:
                line[i] = (line[i] + left) & 0xFF
            elif kind == 2:
                line[i] = (line[i] + up) & 0xFF
            elif kind == 3:
                line[i] = (line[i] + (left + up) // 2) & 0xFF
            elif kind == 4:
                p = left + up - up_left
                pa, pb, pc = abs(p - left), abs(p - up), abs(p - up_left)
                pred = left if pa <= pb and pa <= pc else up if pb <= pc else up_left
                line[i] = (line[i] + pred) & 0xFF
            elif kind != 0:
                sys.exit(f"{path}: unknown filter {kind} on row {y}")
        prev = bytes(line)
        rows.append([tuple(line[x * 4 : x * 4 + 4]) for x in range(width)])
    return rows


def paint(icon: list[list[tuple[int, int, int, int]]]) -> bytes:
    """The icon with the dot and its ring drawn over it, as raw RGBA."""
    out = bytearray()
    cx, cy = CENTRE
    for y in range(SIZE):
        for x in range(SIZE):
            r0, g0, b0, a0 = icon[y][x]
            # Average premultiplied colour over the sub-pixels, so the edges
            # blend into the icon (or into transparency) without fringes.
            acc = [0.0, 0.0, 0.0, 0.0]
            for sy in range(SAMPLES):
                for sx in range(SAMPLES):
                    px = x + (sx + 0.5) / SAMPLES
                    py = y + (sy + 0.5) / SAMPLES
                    d = ((px - cx) ** 2 + (py - cy) ** 2) ** 0.5
                    if d <= DOT_RADIUS:
                        r, g, b, a = (*RED, 1.0)
                    elif d <= RING_RADIUS:
                        r, g, b, a = (*LIGHT, 1.0)
                    else:
                        r, g, b, a = r0, g0, b0, a0 / 255
                    acc[0] += r * a
                    acc[1] += g * a
                    acc[2] += b * a
                    acc[3] += a
            n = SAMPLES * SAMPLES
            alpha = acc[3] / n
            if alpha == 0:
                out += bytes(4)
                continue
            out += bytes(
                [
                    min(255, round(acc[0] / acc[3])),
                    min(255, round(acc[1] / acc[3])),
                    min(255, round(acc[2] / acc[3])),
                    min(255, round(alpha * 255)),
                ]
            )
    return bytes(out)


def main() -> None:
    icon = read_png_rgba(SOURCE)
    if len(icon) != SIZE or any(len(row) != SIZE for row in icon):
        sys.exit(f"{SOURCE}: expected {SIZE}×{SIZE}")
    rgba = paint(icon)
    assert len(rgba) == SIZE * SIZE * 4
    OUTPUT.write_bytes(rgba)
    print(f"wrote {OUTPUT.relative_to(ROOT)} ({len(rgba)} bytes)")


if __name__ == "__main__":
    main()
