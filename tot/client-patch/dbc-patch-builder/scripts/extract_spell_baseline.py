#!/usr/bin/env python3
"""One-time tool: extract the 54 marker-spell baseline rows from a stock Spell.dbc into TSV.

Usage: python extract_spell_baseline.py <input_spell_dbc> <output_tsv>

Output TSV format:
  spell_id<TAB>raw_hex<TAB>name_offset_byte<TAB>desc_offset_byte

The 54 marker spell IDs are sourced from bracket_set_bonus_map_seed.sql.
"""
from __future__ import annotations
import argparse
import re
import struct
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
BONUS_SEED = REPO_ROOT / "modules" / "mod-bracket-sets" / "data" / "sql" / "world" / "2026_05_13_01_bracket_set_bonus_map_seed.sql"

# 3.3.5a Spell.dbc record layout (empirically verified):
SPELL_RECORD_SIZE = 936
SPELL_FIELD_COUNT = 234
NAME_LANG_ENUS_OFFSET = 544   # byte offset within record for SpellName_Lang_enUS string-offset field
DESC_LANG_ENUS_OFFSET = 680   # byte offset within record for SpellDescription_Lang_enUS string-offset field


def extract_marker_ids() -> list[int]:
    """Parse bracket_set_bonus_map_seed.sql for the 54 spell_ids."""
    text = BONUS_SEED.read_text()
    # Match: (90101, threshold, class, spec, spell_id, 25, 34, 'name'),
    pattern = re.compile(r"\(90101,\s*[24],\s*\d+,\s*\d+,\s*(\d+),")
    return sorted({int(m.group(1)) for m in pattern.finditer(text)})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("input_dbc", type=Path, help="Path to stock 3.3.5a Spell.dbc")
    parser.add_argument("output_tsv", type=Path, help="Output baseline TSV path")
    args = parser.parse_args()

    marker_ids = extract_marker_ids()
    assert len(marker_ids) == 54, f"expected 54 marker IDs, got {len(marker_ids)}"
    print(f"Looking up {len(marker_ids)} marker spell IDs in {args.input_dbc}")

    with args.input_dbc.open("rb") as f:
        data = f.read()

    magic, count, fc, rs, sbs = struct.unpack("<4sIIII", data[:20])
    assert magic == b"WDBC", f"not a WDBC file: {magic}"
    assert rs == SPELL_RECORD_SIZE, f"unexpected record_size {rs}, expected {SPELL_RECORD_SIZE}"
    assert fc == SPELL_FIELD_COUNT, f"unexpected field_count {fc}, expected {SPELL_FIELD_COUNT}"

    records_start = 20
    id_to_offset = {}
    for i in range(count):
        rec_off = records_start + i * rs
        spell_id = struct.unpack_from("<I", data, rec_off)[0]
        id_to_offset[spell_id] = rec_off

    args.output_tsv.parent.mkdir(parents=True, exist_ok=True)
    found = 0
    missing = []
    with args.output_tsv.open("w") as out:
        out.write("# spell_id\traw_hex\tname_offset_byte\tdesc_offset_byte\n")
        out.write(f"# Extracted from 3.3.5a Spell.dbc (record_size={SPELL_RECORD_SIZE}, field_count={SPELL_FIELD_COUNT})\n")
        out.write(f"# Name field at byte offset {NAME_LANG_ENUS_OFFSET}, Description field at byte offset {DESC_LANG_ENUS_OFFSET}\n")
        for sid in marker_ids:
            if sid not in id_to_offset:
                missing.append(sid)
                continue
            rec_off = id_to_offset[sid]
            rec = data[rec_off:rec_off + rs]
            out.write(f"{sid}\t{rec.hex()}\t{NAME_LANG_ENUS_OFFSET}\t{DESC_LANG_ENUS_OFFSET}\n")
            found += 1

    print(f"Wrote {found}/{len(marker_ids)} rows to {args.output_tsv}")
    if missing:
        print(f"MISSING: {missing}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
