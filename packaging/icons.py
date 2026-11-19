#!/usr/bin/env python3
"""Makes the installers' icons from the mark (crates/app/assets/cuprita.svg): Windows'
packaging/windows/cuprita.ico and macOS's packaging/macos/cuprita.icns, each holding PNGs, which
both take. ImageMagick (`magick`) draws them; below 24 px the mark drops its vias (BRAND.md).
Run it from anywhere after the mark changes, and commit what it writes."""

import re
import struct
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MARK = ROOT / "crates/app/assets/cuprita.svg"


def pngs(sizes):
    """Each size's PNG of the mark, as bytes."""
    full = MARK.read_text()
    small = re.sub(r'\s*<g fill="#(14100C|E8864F)">.*?</g>', "", full, flags=re.S)
    out = {}
    with tempfile.TemporaryDirectory() as tmp:
        for size in sizes:
            svg = Path(tmp) / f"{size}.svg"
            svg.write_text(small if size < 24 else full)
            png = Path(tmp) / f"{size}.png"
            # Drawn at twice the size, then scaled down: smoother edges.
            density = 72 * size * 2 // 256
            subprocess.run(["magick", "-background", "none", "-density", str(density), str(svg),
                            "-resize", f"{size}x{size}", f"png32:{png}"], check=True)
            out[size] = png.read_bytes()
    return out


def ico(images):
    """An .ico of PNG entries (Windows Vista on), smallest first."""
    head = struct.pack("<HHH", 0, 1, len(images))
    offset, entries, data = 6 + 16 * len(images), b"", b""
    for size, png in sorted(images.items()):
        side = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(png), offset + len(data))
        data += png
    return head + entries + data


# The .icns types that hold a PNG of each size (macOS 10.7 on).
ICNS = [("icp4", 16), ("icp5", 32), ("ic11", 32), ("ic12", 64), ("ic07", 128), ("ic13", 256),
        ("ic08", 256), ("ic14", 512), ("ic09", 512), ("ic10", 1024)]


def icns(images):
    """An .icns of PNG entries."""
    body = b"".join(kind.encode() + struct.pack(">I", 8 + len(images[size])) + images[size] for kind, size in ICNS)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


if __name__ == "__main__":
    images = pngs([16, 24, 32, 48, 64, 128, 256, 512, 1024])
    (ROOT / "packaging/windows").mkdir(exist_ok=True)
    (ROOT / "packaging/macos").mkdir(exist_ok=True)
    (ROOT / "packaging/windows/cuprita.ico").write_bytes(ico({s: images[s] for s in (16, 24, 32, 48, 64, 128, 256)}))
    (ROOT / "packaging/macos/cuprita.icns").write_bytes(icns(images))
    print("wrote packaging/windows/cuprita.ico and packaging/macos/cuprita.icns")
