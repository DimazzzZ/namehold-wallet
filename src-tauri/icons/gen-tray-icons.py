#!/usr/bin/env python3
"""Generate the three monochrome tray template icons from logo.png.

The Namehold logo has no vector source, so its PNG alpha channel is the
authoritative silhouette. This script tight-crops the mark and renders three
44x44 black-on-transparent template images (macOS inverts them automatically
via set_icon_as_template(true)). Re-run whenever logo.png changes.

Usage:  python3 src-tauri/icons/gen-tray-icons.py
Requires: Pillow  (pip install pillow)
"""
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

HERE = Path(__file__).resolve().parent
SRC = HERE / "logo.png"
CANVAS = 44
PAD = 2
CONTENT = CANVAS - 2 * PAD


def _fit(mask: Image.Image, box: int) -> Image.Image:
    """Scale a grayscale mask into a box x box area, centered on CANVAS."""
    r = min(box / mask.width, box / mask.height)
    nw, nh = max(1, round(mask.width * r)), max(1, round(mask.height * r))
    scaled = mask.resize((nw, nh), Image.LANCZOS)
    canvas = Image.new("L", (CANVAS, CANVAS), 0)
    canvas.paste(scaled, ((CANVAS - nw) // 2, (CANVAS - nh) // 2))
    return canvas


def _black(mask: Image.Image) -> Image.Image:
    """Solid-black RGBA using `mask` as the alpha channel."""
    out = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    black = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 255))
    return Image.composite(black, out, mask)


def main() -> None:
    logo = Image.open(SRC).convert("RGBA")
    alpha = logo.split()[3]
    mark = alpha.crop(alpha.getbbox())

    # normal: filled silhouette
    m_norm = _fit(mark, CONTENT)
    _black(m_norm).save(HERE / "tray-normal.png")

    # stopped: outline only (silhouette minus eroded interior)
    m_bin = m_norm.point(lambda v: 255 if v >= 128 else 0)
    eroded = m_bin.filter(ImageFilter.MinFilter(5))
    outline = ImageChops.multiply(m_bin, ImageChops.invert(eroded))
    _black(outline).save(HERE / "tray-stopped.png")

    # syncing: slightly smaller silhouette + activity dot bottom-right
    sync = _black(_fit(mark, CONTENT - 6))
    draw = ImageDraw.Draw(sync)
    r = 6
    cx = cy = CANVAS - PAD - r + 1
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(0, 0, 0, 255))
    sync.save(HERE / "tray-syncing.png")

    print("wrote tray-normal.png, tray-stopped.png, tray-syncing.png")


if __name__ == "__main__":
    main()
