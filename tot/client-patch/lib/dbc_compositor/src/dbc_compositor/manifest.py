# SPDX-License-Identifier: GPL-2.0-or-later
"""MANIFEST.toml loader + RangeRegistry (DBC ID-range collision check A).

A module declares its client DBC contribution in modules/<mod>/client/MANIFEST.toml:

    [manifest]
    mod = "mod-bracket-sets"
    [sources]                       # paths relative to the MODULE ROOT (client/..)
    bonus_map = "data/sql/world/....sql"
    [recipe]
    entry = "build_dbc"             # importable module next to MANIFEST.toml
    [id_ranges]                     # per-DBC-file inclusive ranges; a list allows disjoint clusters
    "ItemSet.dbc" = [{ min = 90100, max = 90199 }]
    "Spell.dbc"   = [{ min = 64854, max = 64939 }, { min = 67121, max = 67268 }]

RangeRegistry ingests every module's [id_ranges] and raises CollisionError on
any overlap for the same DBC filename. Ranges for DIFFERENT files never collide.
"""
from __future__ import annotations

import tomllib
from dataclasses import dataclass, field
from pathlib import Path


class CollisionError(Exception):
    """Raised when two modules claim overlapping DBC ID ranges for one file."""


@dataclass
class Manifest:
    mod: str
    recipe_entry: str
    sources: dict[str, Path]
    id_ranges: dict[str, list[tuple[int, int]]]
    manifest_path: Path
    module_root: Path  # parent of client/


def load_manifest(manifest_path: Path) -> Manifest:
    manifest_path = Path(manifest_path).resolve()
    with open(manifest_path, "rb") as f:
        data = tomllib.load(f)

    mod = data["manifest"]["mod"]
    recipe_entry = data.get("recipe", {}).get("entry", "build_dbc")

    # client/MANIFEST.toml -> module root is two levels up
    module_root = manifest_path.parent.parent

    sources: dict[str, Path] = {}
    for key, rel in data.get("sources", {}).items():
        resolved = (module_root / rel).resolve()
        if not resolved.exists():
            raise FileNotFoundError(
                f"{manifest_path}: [sources].{key} -> {rel} does not exist "
                f"(resolved {resolved})"
            )
        sources[key] = resolved

    id_ranges: dict[str, list[tuple[int, int]]] = {}
    for dbc_name, ranges in data.get("id_ranges", {}).items():
        id_ranges[dbc_name] = [(int(r["min"]), int(r["max"])) for r in ranges]

    return Manifest(
        mod=mod,
        recipe_entry=recipe_entry,
        sources=sources,
        id_ranges=id_ranges,
        manifest_path=manifest_path,
        module_root=module_root,
    )


@dataclass
class RangeRegistry:
    # dbc_name -> list of (mod, min, max)
    _claims: dict[str, list[tuple[str, int, int]]] = field(default_factory=dict)

    def add(self, mod: str, dbc_name: str, ranges: list[tuple[int, int]]) -> None:
        existing = self._claims.setdefault(dbc_name, [])
        for (lo, hi) in ranges:
            for (other_mod, olo, ohi) in existing:
                if lo <= ohi and olo <= hi:  # interval overlap
                    overlap_lo, overlap_hi = max(lo, olo), min(hi, ohi)
                    raise CollisionError(
                        f"DBC ID-range collision in {dbc_name}\n"
                        f"  {other_mod} claims [{olo}, {ohi}]\n"
                        f"  {mod} claims [{lo}, {hi}]\n"
                        f"  overlap: [{overlap_lo}, {overlap_hi}]\n"
                        f"  Resolve by editing [id_ranges] in one module's "
                        f"client/MANIFEST.toml "
                        f"(allocation map: tot/client-patch/DBC-RANGES.md)"
                    )
            existing.append((mod, lo, hi))
