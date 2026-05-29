# SPDX-License-Identifier: GPL-2.0-or-later
"""Priority-based FrameXML override resolution.

A module may ship raw FrameXML/GlueXML file overrides via [framexml_overrides]
in its MANIFEST.toml:

    [[framexml_overrides]]
    src = "client/framexml/GlueParent.xml"    # relative to module root
    dest = "Interface/GlueXML/GlueParent.xml"  # path inside the MPQ
    priority = 50

When two modules target the same dest, the higher priority wins. Equal
priority on the same dest is a hard error (never silent last-write-wins).
No module ships overrides today; this exists for future use and as the
branding fallback rail if GlueXML AddOns do not load on a client build.
"""
from __future__ import annotations


class FrameXmlConflictError(Exception):
    """Raised when two equal-priority overrides target the same dest."""


def resolve_overrides(entries: list[dict]) -> dict[str, str]:
    """entries: list of {mod, src, dest, priority} -> {dest: winning_src}."""
    best: dict[str, dict] = {}
    for e in entries:
        dest = e["dest"]
        cur = best.get(dest)
        if cur is None:
            best[dest] = e
        elif e["priority"] > cur["priority"]:
            best[dest] = e
        elif e["priority"] == cur["priority"]:
            raise FrameXmlConflictError(
                f"FrameXML override conflict on {dest}: "
                f"{cur['mod']} and {e['mod']} both claim priority "
                f"{e['priority']}. Give one a higher priority to resolve."
            )
        # else: keep the existing higher-priority winner
    return {dest: e["src"] for dest, e in best.items()}
