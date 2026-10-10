"""Draws the desktop app's launcher icons from assets/brand/mark.svg.

    python3 assets/brand/tools/build_app_icons.py

Needs rsvg-convert and Pillow. The tile in mark.svg runs edge to edge, which
suits a favicon or a Windows taskbar but looks oversized in a Linux dock or the
macOS Dock, where icons sit on a grid with a margin. So:

    app/src-tauri/icons/{32x32,64x64,128x128,256x256,512x512}.png and icon.png
        Linux launcher and window icons: the tile at 7/8 of the canvas,
        centred, the margin GNOME and KDE icons keep.
    app/src-tauri/icons/icon.icns
        macOS: the tile at 824/1024 of the canvas, Apple's icon grid, with
        the soft shadow Dock icons carry.
    app/src-tauri/icons/icon.ico is left as build_brand.py draws it, full
        bleed and pixel-fitted at 16 and 32 px for the Windows taskbar.
"""

import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageFilter

ROOT = Path(__file__).resolve().parents[3]
MARK = ROOT / "assets/brand/mark.svg"
ICONS = ROOT / "app/src-tauri/icons"

LINUX_SIZES = [32, 64, 128, 256, 512]
LINUX_FILL = 7 / 8
MAC_BODY = 824 / 1024


def raster(size: int, tmp: Path) -> Image.Image:
    """The tile alone, rendered from the vector at this many pixels."""
    out = tmp / f"tile-{size}.png"
    subprocess.run(
        ["rsvg-convert", "-w", str(size), "-h", str(size), "-o", str(out), str(MARK)],
        check=True,
    )
    return Image.open(out).convert("RGBA")


def padded(canvas: int, fill: float, tmp: Path, shadow: bool = False) -> Image.Image:
    body = round(canvas * fill)
    # Keep the body on whole pixels and centred.
    body -= (canvas - body) % 2
    tile = raster(body, tmp)
    img = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    off = (canvas - body) // 2
    if shadow:
        # A soft drop shadow, offset down, as macOS draws under Dock icons.
        alpha = tile.getchannel("A").point(lambda a: a * 0.30)
        shade = Image.new("RGBA", tile.size, (0, 0, 0, 0))
        shade.putalpha(alpha)
        layer = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
        layer.paste(shade, (off, off + round(canvas * 0.012)), shade)
        img = Image.alpha_composite(img, layer.filter(ImageFilter.GaussianBlur(canvas * 0.012)))
    img.alpha_composite(tile, (off, off))
    return img


def main() -> None:
    with tempfile.TemporaryDirectory() as t:
        tmp = Path(t)
        for size in LINUX_SIZES:
            padded(size, LINUX_FILL, tmp).save(ICONS / f"{size}x{size}.png")
        padded(512, LINUX_FILL, tmp).save(ICONS / "icon.png")
        mac = padded(1024, MAC_BODY, tmp, shadow=True)
        mac.save(
            ICONS / "icon.icns",
            sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)],
        )
    # Superseded by 256x256.png: Tauri installed it into a nonstandard
    # 256x256@2 folder that icon themes do not read.
    (ICONS / "128x128@2x.png").unlink(missing_ok=True)
    print(f"wrote launcher icons in {ICONS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
