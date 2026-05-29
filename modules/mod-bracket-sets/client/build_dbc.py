# SPDX-License-Identifier: GPL-2.0-or-later
"""mod-bracket-sets client DBC recipe.

Composes the Bracket 1 tier-set client DBCs (ItemSet.dbc + Spell.dbc) from the
module's authored sources. Exposes the MPQ-compositor recipe contract:

    build(sources: dict[str, Path]) -> {dbc_name: bytes}

`sources` is supplied by the compositor from this module's MANIFEST.toml
[sources] table (already resolved to absolute paths). Expected keys:
    bonus_map      -> bracket_set_bonus_map seed SQL (54 rows)
    itemset_map    -> bracket_set_itemset_map seed SQL (27 (class,spec)->itemset)
    descriptions   -> bracket_set_descriptions.tsv (54 rows: flavor + mechanic)
    spell_baseline -> spell_dbc_baseline.tsv (54 raw 936-byte records)

The output blobs are byte-locked by golden/ regression anchors captured from the
legacy dbc-patch-builder (itemset sha=2a5685d6, spell sha=476cedea). Any change
to the compose logic or the tables below WILL break the golden tests — that is
the intended safety net.
"""
from __future__ import annotations

import csv
import re
from pathlib import Path
from typing import Dict, List

from dbc_compositor.bonus_map_parser import parse_bonus_map_seed
from dbc_compositor.itemset_dbc import (
    build_itemset_dbc,
    compose_27_class_spec_rows,
    fallback_row,
)
from dbc_compositor.spell_dbc import (
    SpellOverride,
    build_spell_dbc_overrides,
    load_baseline,
)

BRACKET_WINDOW_LINE = "Active while in Bracket 1 (L25-34)"

# Member items per armor type. Queried from item_template WHERE itemset=90101
# on 2026-05-25. Each class's set is composed of (class-appropriate armor) + misc.
# The ItemSet.dbc itemId[17] field must contain at least one non-zero entry or the
# 3.3.5a client treats the set as malformed and skips bonus rendering.
# These tables are copied verbatim from the legacy dbc-patch-builder build.py;
# they are load-bearing for the golden blobs and MUST NOT be reordered/edited.
_ITEMS_MISC    = [90004, 90005, 90009, 90020, 90025, 90037, 90039, 90044, 90048]   # neck/ring/cloak/trinket (9)
_ITEMS_CLOTH   = [90007, 90038, 90040, 90041, 90042, 90043, 90046, 90051, 90201, 90205]  # 10
_ITEMS_LEATHER = [90003, 90010, 90012, 90024, 90027, 90033, 90049, 90203]               # 8
_ITEMS_MAIL    = [90000, 90002, 90021, 90023, 90036, 90047, 90202]                      # 7
_ITEMS_PLATE   = [90200, 90204, 90206]                                                  # 3

# class_id -> ordered item list for itemId[17] (armor-type-specific first, then misc).
# Capped at 17 by _pad_items in itemset_dbc.py.
CLASS_ITEMS = {
    1:  _ITEMS_PLATE   + _ITEMS_MISC,  # Warrior
    2:  _ITEMS_PLATE   + _ITEMS_MISC,  # Paladin
    3:  _ITEMS_MAIL    + _ITEMS_MISC,  # Hunter
    4:  _ITEMS_LEATHER + _ITEMS_MISC,  # Rogue
    5:  _ITEMS_CLOTH   + _ITEMS_MISC,  # Priest
    6:  _ITEMS_PLATE   + _ITEMS_MISC,  # Death Knight
    7:  _ITEMS_MAIL    + _ITEMS_MISC,  # Shaman
    8:  _ITEMS_CLOTH   + _ITEMS_MISC,  # Mage
    9:  _ITEMS_CLOTH   + _ITEMS_MISC,  # Warlock
    11: _ITEMS_LEATHER + _ITEMS_MISC,  # Druid
}


def _load_descriptions(tsv_path: Path) -> Dict[int, tuple[str, str]]:
    """Parse the descriptions TSV: spell_id -> (flavor, mechanic)."""
    out: Dict[int, tuple[str, str]] = {}
    with Path(tsv_path).open() as f:
        reader = csv.reader(f, delimiter="\t")
        for row in reader:
            if not row:
                continue
            if row[0].startswith("#"):
                continue
            if row[0] == "spell_id":  # header
                if row != ["spell_id", "flavor", "mechanic"]:
                    raise ValueError(f"unexpected descriptions TSV header: {row}")
                continue
            spell_id = int(row[0])
            out[spell_id] = (row[1], row[2])
    return out


def _parse_itemset_map_seed(sql_path: Path) -> Dict[tuple[int, int], int]:
    """Pull (class, spec) -> itemset_id from the seed SQL for Bracket 1 (bracket_id=1)."""
    text = Path(sql_path).read_text()
    out: Dict[tuple[int, int], int] = {}
    # Match: ( cls, spec, 1, itemset_id), tolerating whitespace
    for m in re.finditer(r"\(\s*(\d+)\s*,\s*(\d+)\s*,\s*1\s*,\s*(\d+)\s*\)", text):
        cls, spec, itemset = int(m.group(1)), int(m.group(2)), int(m.group(3))
        out[(cls, spec)] = itemset
    return out


def build(sources: Dict[str, Path]) -> Dict[str, bytes]:
    """Compose the Bracket 1 client DBCs.

    Returns {"ItemSet.dbc": <bytes>, "Spell.dbc": <bytes>} — the patch-local
    override blocks the compositor packs into the unified patch MPQ.
    """
    bonus_rows = parse_bonus_map_seed(sources["bonus_map"])
    if len(bonus_rows) != 54:
        raise ValueError(f"expected 54 bonus rows, got {len(bonus_rows)}")

    itemset_map = _parse_itemset_map_seed(sources["itemset_map"])
    if len(itemset_map) != 27:
        raise ValueError(f"expected 27 itemset rows, got {len(itemset_map)}")

    descriptions = _load_descriptions(sources["descriptions"])
    if len(descriptions) != 54:
        raise ValueError(f"expected 54 description rows, got {len(descriptions)}")

    baseline = load_baseline(sources["spell_baseline"])
    if len(baseline) != 54:
        raise ValueError(f"expected 54 baseline rows, got {len(baseline)}")

    # Compose ItemSet.dbc — 28 rows (1 fallback + 27 class+spec)
    itemset_rows = [fallback_row()] + compose_27_class_spec_rows(
        bonus_rows, itemset_map, class_items=CLASS_ITEMS
    )
    itemset_blob = build_itemset_dbc(itemset_rows)

    # Compose Spell.dbc overrides (54 rows). Order by spell_id for deterministic output.
    name_by_spell = {r.spell_id: r.display_name for r in bonus_rows}
    overrides: List[SpellOverride] = []
    for spell_id in sorted(descriptions.keys()):
        flavor, mechanic = descriptions[spell_id]
        overrides.append(SpellOverride(
            spell_id=spell_id,
            name=name_by_spell[spell_id],
            description=f"{flavor}\r{mechanic}\r{BRACKET_WINDOW_LINE}",
        ))
    spell_blob = build_spell_dbc_overrides(baseline, overrides)

    return {"ItemSet.dbc": itemset_blob, "Spell.dbc": spell_blob}
