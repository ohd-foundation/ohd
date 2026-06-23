#!/usr/bin/env python3
"""Rewrite each built SVG's viewBox to tightly bound its content.

`extract.py` produces standalone SVGs that reuse the source's
shared canvas viewBox, so each logo renders correctly but lives in
a corner of a much larger empty box. This script uses Inkscape's
``--query-*`` mode to compute the actual drawing bbox of every file
and rewrites the viewBox in-place.

Inkscape only honours queries when an interactive display isn't
demanded — `env DISPLAY= inkscape …` is enough to keep it from
trying to talk to X. The query output is parsed line-by-line; the
``svg<n>,…`` line carries the document's full drawing bbox, which is
what we want since each file holds a single logo group.

Re-runnable. A file whose viewBox already matches the queried bbox
is rewritten identically; lxml's pretty-printer is the only
non-idempotent thing in the round-trip (whitespace shifts).
"""

from __future__ import annotations
import os
import subprocess
import sys
from pathlib import Path
import lxml.etree as ET

SVG_NS = "http://www.w3.org/2000/svg"


def query_bbox(svg_path: Path) -> tuple[float, float, float, float] | None:
    """Return (x, y, w, h) of the drawing area, or None if Inkscape fails."""
    env = {**os.environ, "DISPLAY": ""}
    r = subprocess.run(
        ["inkscape", "--query-all", str(svg_path)],
        capture_output=True,
        text=True,
        env=env,
    )
    if r.returncode != 0:
        return None
    # First line ("svg1,…") is the root document's bbox.
    for line in r.stdout.splitlines():
        parts = line.split(",")
        if len(parts) != 5 or not parts[0].startswith("svg"):
            continue
        try:
            return tuple(float(p) for p in parts[1:])  # type: ignore[return-value]
        except ValueError:
            return None
    return None


def tighten(svg_path: Path) -> bool:
    """Tighten one file. Returns True iff the viewBox changed."""
    bbox = query_bbox(svg_path)
    if bbox is None:
        print(f"  ! {svg_path.name}: bbox query failed; viewBox kept", file=sys.stderr)
        return False
    x, y, w, h = bbox
    # Tiny breathing room (1% of the larger side) so strokes near the
    # edge aren't clipped at small render sizes. Below 1px we don't pad.
    pad = max(0.0, max(w, h) * 0.01)
    new_view = f"{x - pad:.3f} {y - pad:.3f} {w + 2 * pad:.3f} {h + 2 * pad:.3f}"

    tree = ET.parse(str(svg_path))
    root = tree.getroot()
    old_view = root.get("viewBox")
    if old_view == new_view:
        return False
    root.set("viewBox", new_view)
    # Drop width/height attrs if any; viewBox alone is enough and lets
    # consumers scale freely via CSS.
    for attr in ("width", "height"):
        if root.get(attr):
            del root.attrib[attr]
    tree.write(
        str(svg_path),
        pretty_print=True,
        xml_declaration=True,
        encoding="utf-8",
    )
    return True


def main(built_dir: Path) -> int:
    files = sorted(built_dir.glob("*.svg"))
    if not files:
        print(f"no svgs in {built_dir}/", file=sys.stderr)
        return 1
    changed = 0
    for f in files:
        if tighten(f):
            changed += 1
            print(f"  tightened {f.name}")
        else:
            print(f"  unchanged {f.name}")
    print(f"\n{changed}/{len(files)} files re-tightened")
    return 0


if __name__ == "__main__":
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("built")
    sys.exit(main(out))
