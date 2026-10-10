"""Renames a font family so a modified (subset) font never ships under its original name.

Subsetting is a modification under the SIL Open Font License. None of the three
upstream fonts declares a Reserved Font Name, but the families are renamed
anyway: a subset that still says "Instrument Sans" would be mistaken for the
complete font by anyone who installs it, and renaming costs nothing.

Every record a system could read the family from is rewritten: nameID 1 is what
a font menu shows, 16 and 21 are what newer systems show, 6 is what a PDF
embeds, 25 prefixes the PostScript names of variable instances, and each fvar
instance carries its own PostScript name record.

Three records are left exactly as upstream wrote them, because they are
attribution and the licence exists to preserve them:

    0  copyright notice
    7  trademark
    8  manufacturer

After the targeted rewrites, any record outside those three that still contains
the original family name fails the run instead of producing a file that cannot
be redistributed under the new name.
"""

from fontTools.ttLib import TTFont

ATTRIBUTION_IDS = frozenset({0, 7, 8})


def _instance_names(font: TTFont) -> tuple[frozenset[int], dict[int, int]]:
    """Style and axis name IDs to keep, and fvar PostScript name IDs to rewrite."""
    keep: set[int] = set()
    postscript: dict[int, int] = {}
    if "fvar" not in font:
        return frozenset(keep), postscript
    for axis in font["fvar"].axes:
        keep.add(axis.axisNameID)
    for instance in font["fvar"].instances:
        keep.add(instance.subfamilyNameID)
        ps_id = getattr(instance, "postscriptNameID", 0xFFFF)
        if ps_id != 0xFFFF:
            postscript[ps_id] = instance.subfamilyNameID
    return frozenset(keep), postscript


def rename(font: TTFont, family: str, original: str, style: str = "Regular") -> None:
    compact = family.replace(" ", "")
    original_compact = original.replace(" ", "")
    name = font["name"]
    keep_ids, postscript_ids = _instance_names(font)

    def style_of(subfamily_id: int) -> str:
        return (name.getDebugName(subfamily_id) or "Regular").replace(" ", "")

    for record in list(name.names):
        nid = record.nameID
        if nid in ATTRIBUTION_IDS:
            continue
        where = (record.platformID, record.platEncID, record.langID)
        if nid in postscript_ids:
            name.setName(f"{compact}-{style_of(postscript_ids[nid])}", nid, *where)
        elif nid == 1:
            # The legacy family groups at most four styles (regular, bold and
            # their italics); anything else carries its style in the family name,
            # with nameID 16 and 17 holding the real family and style.
            ribbi = style in ("Regular", "Bold", "Italic", "Bold Italic")
            name.setName(family if ribbi else f"{family} {style}", nid, *where)
        elif nid in (16, 21):
            name.setName(family, nid, *where)
        elif nid in (4, 18):
            name.setName(family if style == "Regular" else f"{family} {style}", nid, *where)
        elif nid == 3:
            name.setName(f"{family} {style};aispice", nid, *where)
        elif nid == 6:
            name.setName(f"{compact}-{style.replace(' ', '')}", nid, *where)
        elif nid == 25:
            name.setName(compact, nid, *where)
        elif nid not in keep_ids:
            text = str(record)
            swept = text.replace(original, family).replace(original_compact, compact)
            if swept != text:
                name.setName(swept, nid, *where)

    if "CFF " in font:
        cff = font["CFF "].cff
        cff.fontNames = [f"{compact}-{style.replace(' ', '')}"]
        top = cff.topDictIndex[0]
        for key in ("FullName", "FamilyName"):
            if hasattr(top, key):
                setattr(top, key, family if key == "FamilyName" else f"{family} {style}")

    survivors = [
        (r.nameID, str(r))
        for r in name.names
        if r.nameID not in ATTRIBUTION_IDS and (original in str(r) or original_compact in str(r))
    ]
    if survivors:
        lines = "\n".join(f"  nameID {nid}: {text!r}" for nid, text in survivors)
        raise SystemExit(f"reserved name survives after renaming to {family!r}:\n{lines}")
