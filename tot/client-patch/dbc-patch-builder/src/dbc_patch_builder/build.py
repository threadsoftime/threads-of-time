# tools/dbc-patch-builder/src/dbc_patch_builder/build.py
"""End-to-end builder: bonus_map_seed + descriptions + baseline -> patch-Z.MPQ + migration SQL."""
from __future__ import annotations
import argparse
import csv
import sys
from pathlib import Path
from typing import Dict, List

from .bonus_map_parser import parse_bonus_map_seed
from .itemset_dbc import build_itemset_dbc, compose_27_class_spec_rows, fallback_row
from .spell_dbc import SpellOverride, load_baseline, build_spell_dbc_overrides
from .mpq_pack import pack_mpq

# Repo-relative paths (this file lives at tools/dbc-patch-builder/src/dbc_patch_builder/)
REPO_ROOT = Path(__file__).resolve().parents[4]
BONUS_SEED = REPO_ROOT / "modules" / "mod-bracket-sets" / "data" / "sql" / "world" / "2026_05_13_01_bracket_set_bonus_map_seed.sql"
DESCRIPTIONS = REPO_ROOT / "modules" / "mod-bracket-sets" / "data" / "fixtures" / "bracket_set_descriptions.tsv"
ITEMSET_MAP_SEED = REPO_ROOT / "modules" / "mod-bracket-sets" / "data" / "sql" / "world" / "2026_05_23_07_bracket_set_itemset_map_seed.sql"
BASELINE = REPO_ROOT / "tools" / "dbc-patch-builder" / "baseline" / "spell_dbc_baseline.tsv"
BUILD_DIR = REPO_ROOT / "tools" / "dbc-patch-builder" / "build"
MPQ_OUT = BUILD_DIR / "patch-Z.MPQ"

BRACKET_WINDOW_LINE = "Active while in Bracket 1 (L25-34)"

# Member items per armor type. Queried from item_template WHERE itemset=90101
# on 2026-05-25. Each class's set is composed of (class-appropriate armor) + misc.
# The ItemSet.dbc itemId[17] field must contain at least one non-zero entry or the
# 3.3.5a client treats the set as malformed and skips bonus rendering.
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


def load_descriptions(tsv_path: Path) -> Dict[int, tuple[str, str]]:
    """Parse the descriptions TSV: spell_id -> (flavor, mechanic)."""
    out: Dict[int, tuple[str, str]] = {}
    with tsv_path.open() as f:
        reader = csv.reader(f, delimiter="\t")
        for row in reader:
            if not row:
                continue
            if row[0].startswith("#"):
                continue
            if row[0] == "spell_id":  # header
                assert row == ["spell_id", "flavor", "mechanic"], f"unexpected header {row}"
                continue
            spell_id = int(row[0])
            out[spell_id] = (row[1], row[2])
    return out


def parse_itemset_map_seed(sql_path: Path) -> Dict[tuple[int, int], int]:
    """Pull (class, spec) -> itemset_id from the seed SQL for Bracket 1 (bracket_id=1)."""
    import re
    text = sql_path.read_text()
    out: Dict[tuple[int, int], int] = {}
    # Match: ( cls, spec, 1, itemset_id), tolerating whitespace
    for m in re.finditer(r"\(\s*(\d+)\s*,\s*(\d+)\s*,\s*1\s*,\s*(\d+)\s*\)", text):
        cls, spec, itemset = int(m.group(1)), int(m.group(2)), int(m.group(3))
        out[(cls, spec)] = itemset
    return out


def main(argv: List[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Build patch-Z.MPQ for Bracket 1 tier-set UI")
    parser.add_argument("--out", type=Path, default=MPQ_OUT, help="Output MPQ path")
    args = parser.parse_args(argv)

    print(f"Reading bonus map: {BONUS_SEED}")
    bonus_rows = parse_bonus_map_seed(BONUS_SEED)
    assert len(bonus_rows) == 54, f"expected 54 bonus rows, got {len(bonus_rows)}"

    print(f"Reading itemset map: {ITEMSET_MAP_SEED}")
    itemset_map = parse_itemset_map_seed(ITEMSET_MAP_SEED)
    assert len(itemset_map) == 27, f"expected 27 itemset rows, got {len(itemset_map)}"

    print(f"Reading descriptions: {DESCRIPTIONS}")
    descriptions = load_descriptions(DESCRIPTIONS)
    assert len(descriptions) == 54, f"expected 54 description rows, got {len(descriptions)}"

    print(f"Reading Spell.dbc baseline: {BASELINE}")
    baseline = load_baseline(BASELINE)
    assert len(baseline) == 54, f"expected 54 baseline rows, got {len(baseline)}"

    # Compose ItemSet.dbc — 28 rows (1 fallback + 27 class+spec)
    print("Composing ItemSet.dbc (28 rows)")
    itemset_rows = [fallback_row()] + compose_27_class_spec_rows(
        bonus_rows, itemset_map, class_items=CLASS_ITEMS
    )
    itemset_blob = build_itemset_dbc(itemset_rows)

    # Compose Spell.dbc overrides (54 rows). Order by spell_id for deterministic output.
    print("Composing Spell.dbc overrides (54 rows)")
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

    # Pack MPQ
    print(f"Packing MPQ -> {args.out}")
    pack_mpq(args.out, {
        "DBFilesClient\\ItemSet.dbc": itemset_blob,
        "DBFilesClient\\Spell.dbc": spell_blob,
    })

    import hashlib
    sha = hashlib.sha256(args.out.read_bytes()).hexdigest()
    print(f"DONE: {args.out} ({args.out.stat().st_size} bytes, sha256={sha[:16]}...)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
