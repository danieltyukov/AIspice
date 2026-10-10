"""Encodes the product renders for the site.

    SITE_RENDERS=1 npx playwright test e2e/site.spec.ts   (in app/)
    sciviz python site/tools/encode_renders.py

Reads the PNG captures in site/renders-src/ (not committed) and writes WebP
and AVIF files to site/public/img/app/, each at full size (2x pixel density)
and at half size for small screens. Needs Pillow with WebP and AVIF support.
"""

from pathlib import Path

from PIL import Image, features

SITE = Path(__file__).resolve().parents[1]
SRC = SITE / "renders-src"
OUT = SITE / "public/img/app"


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    avif = features.check("avif")
    for png in sorted(SRC.glob("*.png")):
        img = Image.open(png).convert("RGB")
        half = img.resize((img.width // 2, img.height // 2), Image.LANCZOS)
        for suffix, im in (("", img), ("-1x", half)):
            stem = OUT / f"{png.stem}{suffix}"
            im.save(f"{stem}.webp", quality=88, method=6)
            if avif:
                im.save(f"{stem}.avif", quality=70, speed=4)
        print(f"{png.name}: {img.width}x{img.height}")
    if not avif:
        print("note: this Pillow has no AVIF encoder; wrote WebP only")


if __name__ == "__main__":
    main()
