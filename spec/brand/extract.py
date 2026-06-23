#!/usr/bin/env python3
"""Extract per-variant brand SVGs from the single-source Inkscape file.

Reads `source/branding.svg`, walks every `<g>` carrying an
`inkscape:label` of the form ``<brand>_logo_for_<bg>_bg``, and writes
each one out to ``built/<brand>-on-<bg>.svg`` as a standalone document.

Group labels in the source are the canonical name. Inkscape's id
attribute is auto-generated and read-only in the Object Properties
dialog, so we never depend on it — only on the human-set label.

Why a separate viewBox per file: the source SVG places each variant at
a different (x, y) on a shared canvas (the artboard with all nine
logos laid out side-by-side). Exporting each `<g>` with the *source's*
viewBox would render each output with a lot of empty space and the
logo offset into a corner. We re-tightt the viewBox using each group's
geometry as computed by Inkscape's `--query-*` flags (called by the
Makefile), so the standalone SVGs render at their natural size with no
padding. This script writes a placeholder `viewBox` initially; the
Makefile's `make tight-viewbox` pass rewrites it with the queried
dimensions.

Skipped labels: `Image` (a layout reference), any label starting with
``demo-`` (user-side scratch), and the inkscape `<sodipodi:namedview>`
element (Inkscape editor state, not part of the rendered image).
"""

from __future__ import annotations
import re
import sys
from pathlib import Path
import lxml.etree as ET

SVG_NS = "http://www.w3.org/2000/svg"
INK_NS = "http://www.inkscape.org/namespaces/inkscape"
NS = {"svg": SVG_NS, "inkscape": INK_NS}
LABEL_ATTR = f"{{{INK_NS}}}label"

# Match labels like `ohd_logo_for_white_bg`, `identity_logo_for_red_bg`,
# `cord_logo_for_red_and_gray_background`. The brand is the prefix; the
# background descriptor is the rest. We normalize both halves.
LABEL_RE = re.compile(
    r"^(?P<brand>[a-z]+)_logo_for_(?P<bg>.+?)(?:_bg(?:\s*)|_background\s*)$",
    re.IGNORECASE,
)


def slug(value: str) -> str:
    """Lowercase, replace underscores + spaces with single hyphens."""
    return re.sub(r"[\s_]+", "-", value.strip().lower())


def normalize_bg(bg: str) -> str:
    """`red_and_gray` → `red-and-gray`; `white` → `white`."""
    return slug(bg)


def main(source: Path, out_dir: Path) -> int:
    out_dir.mkdir(parents=True, exist_ok=True)
    tree = ET.parse(str(source))
    root = tree.getroot()

    # Preserve the source's coordinate system so the queried viewBox
    # (run later in the Makefile via inkscape --query-*) lines up.
    source_view = root.get("viewBox") or "0 0 1000 1000"
    written = []

    for g in root.findall(".//svg:g[@inkscape:label]", NS):
        label = (g.get(LABEL_ATTR) or "").strip()
        m = LABEL_RE.match(label)
        if not m:
            continue
        brand = slug(m.group("brand"))
        bg = normalize_bg(m.group("bg"))
        fname = f"{brand}-on-{bg}.svg"

        # Build a fresh document with just this group inside. lxml
        # populates `xmlns="…"` from the nsmap automatically — explicit
        # `.set("xmlns", …)` would duplicate it and trip strict parsers.
        new_root = ET.Element(
            f"{{{SVG_NS}}}svg",
            nsmap={None: SVG_NS, "inkscape": INK_NS},
        )
        new_root.set("viewBox", source_view)
        # Keep the inkscape:label so a future re-export can re-find it.
        new_g = ET.fromstring(ET.tostring(g))
        new_root.append(new_g)

        out_path = out_dir / fname
        ET.ElementTree(new_root).write(
            str(out_path),
            pretty_print=True,
            xml_declaration=True,
            encoding="utf-8",
        )
        written.append(out_path.name)

    if not written:
        print("no matching <g inkscape:label='*_logo_for_*_bg'> found", file=sys.stderr)
        return 1

    print(f"wrote {len(written)} files to {out_dir}/:")
    for name in sorted(written):
        print(f"  {name}")
    return 0


if __name__ == "__main__":
    src = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("source/branding.svg")
    out = Path(sys.argv[2]) if len(sys.argv) > 2 else Path("built")
    sys.exit(main(src, out))
