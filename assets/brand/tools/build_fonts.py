"""Builds the self-hosted font subsets in app/src/fonts from the upstream files.

The output is committed, so this runs rarely: to pick up an upstream release or
to widen the character set. Nothing in the app or site build calls it.

    python3 assets/brand/tools/build_fonts.py \\
        --instrument InstrumentSans[wdth,wght].ttf \\
        --jetbrains JetBrainsMono[wght].ttf \\
        --inter-regular Inter-Regular.otf --inter-semibold Inter-SemiBold.otf \\
        --instrument-ofl OFL.txt --jetbrains-ofl OFL.txt --inter-ofl LICENSE.txt

Upstream sources: google/fonts ofl/instrumentsans and ofl/jetbrainsmono, and
Inter 4.0 from rsms/inter (the Debian fonts-inter package ships the same files).

Three families come out, all renamed because a subset is a modified version
under the SIL Open Font License:

    aispice Sans          Instrument Sans, variable wdth 75-100 and wght 400-700
    aispice Mono          JetBrains Mono, variable wght 400-700
    aispice Sans Symbols  Inter Regular and SemiBold, only the characters
                          Instrument Sans lacks (Greek letters, the micro sign,
                          plus-minus, comparison operators, fractions,
                          superscripts). tokens.css registers these under the
                          CSS family "aispice Sans" with a unicode-range, so a
                          value like 4.7 kΩ ± 1 % renders in one family without
                          the browser falling back to a system font.

Requires fontTools with brotli (pip install 'fonttools[woff]').
"""

import argparse
import io
import shutil
import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
from rename_font import rename  # noqa: E402

REPO = Path(__file__).resolve().parents[3]
OUT = REPO / "app" / "src" / "fonts"


def ranges(spec: str) -> set[int]:
    out: set[int] = set()
    for part in spec.split(","):
        part = part.strip().upper().removeprefix("U+")
        if "-" in part:
            lo, hi = part.split("-")
            out.update(range(int(lo, 16), int(hi, 16) + 1))
        else:
            out.add(int(part, 16))
    return out


# Google's "latin" subset: ASCII, Latin-1, common combining marks, typographic
# punctuation, and U+FFFD so a bad byte still renders as a box.
LATIN = ranges(
    "U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,"
    "U+0329,U+2000-206F,U+2074,U+20AC,U+2122,U+2191,U+2193,U+2212,U+2215,U+FEFF,U+FFFD"
)

# What circuit text uses: units and Greek letters (kΩ, µs, τ, ω, φ, Δ), arrows,
# comparison operators for spec limits, roots and infinity, the dot operator,
# and super and subscript digits (V², R₁). Latin-1 already brings µ ° ± × ½.
ELECTRONICS = ranges(
    "U+0394,U+03A9,U+03B1-03B2,U+03B8,U+03BC,U+03C0,U+03C4,U+03C6,U+03C9,U+2126,"
    "U+2070-209F,U+2190-2199,U+2206,U+221A,U+221E,U+2248,U+2260,U+2264-2265,U+22C5"
)

# Box drawing, for CLI tables and trees shown in the monospace face.
MONO_EXTRA = ranges("U+2500-257F")

SANS_FEATURES = ["kern", "liga", "calt", "ccmp", "locl", "mark", "mkmk", "tnum", "pnum", "case"]
# No calt or liga: JetBrains Mono draws "--" and "->" as ligatures, and a command
# shown on the page must look exactly like what gets pasted into a terminal.
MONO_FEATURES = ["kern", "ccmp", "locl", "mark", "mkmk", "tnum", "zero"]


def add_ohm_sign(font: TTFont) -> None:
    """Map U+2126 OHM SIGN to the Greek capital omega where only the latter exists.

    Unicode canonically decomposes U+2126 to U+03A9, so they are the same letter;
    text pasted from a datasheet often carries the ohm sign.
    """
    for table in font["cmap"].tables:
        if table.isUnicode() and 0x03A9 in table.cmap and 0x2126 not in table.cmap:
            table.cmap[0x2126] = table.cmap[0x03A9]


def reloaded(font: TTFont) -> TTFont:
    """Serialise and parse again. The instancer leaves some tables half-loaded,
    and the subsetter then trips over glyph variations it cannot find."""
    buf = io.BytesIO()
    font.save(buf)
    buf.seek(0)
    return TTFont(buf)


def write_subset(font: TTFont, unicodes: set[int], features: list[str], out: Path) -> set[int]:
    options = subset.Options()
    options.flavor = "woff2"
    options.layout_features = features
    options.name_IDs = ["*"]
    options.drop_tables += ["DSIG"]
    options.hinting = False
    options.desubroutinize = True
    options.notdef_outline = True
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=sorted(unicodes))
    subsetter.subset(font)
    font.flavor = "woff2"
    out.parent.mkdir(parents=True, exist_ok=True)
    font.save(out)
    kept = set(font.getBestCmap())
    print(f"  {out.relative_to(REPO)}  {out.stat().st_size / 1024:.1f} KiB, {len(kept)} characters")
    return kept


def unicode_range(codepoints: set[int]) -> str:
    cps = sorted(codepoints)
    parts, start = [], None
    for i, cp in enumerate(cps):
        if start is None:
            start = cp
        if i + 1 == len(cps) or cps[i + 1] != cp + 1:
            parts.append(f"U+{start:04X}" if start == cp else f"U+{start:04X}-{cp:04X}")
            start = None
    return ", ".join(parts)


def write_static(out: Path, instrument: Path, jetbrains: Path) -> None:
    """Static instances under a "Draw" family, so Inkscape can outline text in the
    exact weights the brand files use without any font being installed."""
    out.mkdir(parents=True, exist_ok=True)
    jobs = [
        (instrument, "Instrument Sans", {"wdth": 100, "wght": 400}, "aispice Sans Draw", "Regular"),
        (instrument, "Instrument Sans", {"wdth": 100, "wght": 500}, "aispice Sans Draw", "Medium"),
        (instrument, "Instrument Sans", {"wdth": 100, "wght": 600}, "aispice Sans Draw", "SemiBold"),
        (jetbrains, "JetBrains Mono", {"wght": 400}, "aispice Mono Draw", "Regular"),
    ]
    for src, original, location, family, style in jobs:
        font = instancer.instantiateVariableFont(TTFont(src), location)
        rename(font, family, original, style=style)
        path = out / f"{family.replace(' ', '-').lower()}-{style.lower()}.ttf"
        font.save(path)
        print(f"  {path} (static, for drawing)")


def main() -> None:
    p = argparse.ArgumentParser()
    for arg in ("instrument", "jetbrains", "inter-regular", "inter-semibold",
                "instrument-ofl", "jetbrains-ofl", "inter-ofl"):
        p.add_argument(f"--{arg}", required=True, type=Path)
    p.add_argument("--static-dir", type=Path,
                   help="also write static TTF instances here for build_brand.py to draw text with; "
                        "they are build inputs, not committed")
    a = p.parse_args()

    if a.static_dir:
        write_static(a.static_dir, a.instrument, a.jetbrains)

    print("aispice Sans (Instrument Sans)")
    sans = TTFont(a.instrument)
    rename(sans, "aispice Sans", "Instrument Sans")
    sans_kept = write_subset(sans, LATIN | ELECTRONICS, SANS_FEATURES, OUT / "sans" / "aispice-sans.woff2")
    shutil.copyfile(a.instrument_ofl, OUT / "sans" / "OFL.txt")

    print("aispice Mono (JetBrains Mono)")
    mono = reloaded(instancer.instantiateVariableFont(TTFont(a.jetbrains), {"wght": (400, 700)}))
    rename(mono, "aispice Mono", "JetBrains Mono")
    add_ohm_sign(mono)
    write_subset(mono, LATIN | ELECTRONICS | MONO_EXTRA, MONO_FEATURES, OUT / "mono" / "aispice-mono.woff2")
    shutil.copyfile(a.jetbrains_ofl, OUT / "mono" / "OFL.txt")

    print("aispice Sans Symbols (Inter)")
    wanted = (LATIN | ELECTRONICS) - sans_kept
    # Controls and format characters have no glyph worth borrowing.
    wanted = {cp for cp in wanted if cp > 0x20 and not 0x7F <= cp <= 0xA0 and cp not in (0xAD, 0xFEFF)}
    covered: set[int] = set()
    for src, weight, style in ((a.inter_regular, 400, "Regular"), (a.inter_semibold, 600, "SemiBold")):
        sym = TTFont(src)
        rename(sym, "aispice Sans Symbols", "Inter", style=style)
        add_ohm_sign(sym)
        covered = write_subset(sym, wanted, ["kern", "ccmp", "locl", "mark", "mkmk", "case"],
                               OUT / "symbols" / f"aispice-sans-symbols-{weight}.woff2")
    shutil.copyfile(a.inter_ofl, OUT / "symbols" / "OFL.txt")

    missing = wanted - covered
    if missing:
        print("  not in any face (left to the system fallback):", unicode_range(missing))
    print("\nunicode-range for the symbols @font-face rules:\n  " + unicode_range(covered))


if __name__ == "__main__":
    main()
