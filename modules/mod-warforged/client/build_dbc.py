# SPDX-License-Identifier: GPL-2.0-or-later
"""mod-warforged client DBC recipe (§10.3-clean partial SpellItemEnchantment.dbc).

Emits a PARTIAL SpellItemEnchantment.dbc containing ONLY the 21 ToT Warforged
enchant rows (IDs 70001..70063). The 3.3.5a client merges this patch DBC per-ID:
for any enchant ID present in the patch, the patch record takes precedence; IDs
absent from the patch fall through to the stock client DBC. So the client never
needs (and must never receive, per §10.3) the 2656 stock Blizzard rows.

The SERVER side is built separately by tools/build-warforged-dbc.py, which APPENDS
the same 21 rows onto a pristine 2656-row stock DBC (because AC's loader reads the
whole file). This recipe deliberately produces a DIFFERENT artifact: the 21-row
delta only. Both derive the per-field enchant VALUES from the SAME CSV via the
SAME column->field mapping, so the client tooltip shows exactly the stats the
server applies.

Recipe contract:
    build(sources: dict[str, Path]) -> {"SpellItemEnchantment.dbc": bytes}

`sources` is supplied by the compositor from this module's MANIFEST.toml
[sources] table (already resolved to absolute paths). Expected key:
    enchants_csv -> data/csv/warforged_enchants.csv (21 calibrated rows)

WotLK 3.3.5a SpellItemEnchantment.dbc schema (38 uint32 fields, 152 bytes/record),
mirrored byte-for-byte from tools/build-warforged-dbc.py (the source of truth):
  0:     ID
  1:     charges                       (0)
  2-4:   effect[3]                     (CSV e*_type; 5 = ITEM_ENCHANTMENT_TYPE_STAT)
  5-7:   effectPointsMin[3]            (CSV e*_amount)
  8-10:  effectPointsMax[3]            (== Min for fixed-value stat enchants)
  11-13: effectArg[3]                  (CSV e*_stat; ITEM_MOD_* id)
  14-29: name_lang[16]                 (all 16 slots -> the "Warforged" string)
  30:    name_lang_flags               (0xFFFFFFFF — all-locales-set)
  31:    itemVisualID                  (0)
  32:    flags                         (0)
  33:    src_itemID                    (0)
  34:    condition_id                  (0)
  35:    required_skill_id             (0)
  36:    required_skill_rank           (0)
  37:    required_level                (0)

CSV layout (14 columns, comment lines start with '#', incl. the commented header):
  id, band, quality, band_label, description,
  e1_type, e1_amount, e1_stat, e2_type, e2_amount, e2_stat, e3_type, e3_amount, e3_stat
"""
from __future__ import annotations

import csv
import struct
from pathlib import Path
from typing import Dict, List

from dbc_compositor.dbc_io import StringBlock, pack_header

# DBC format constants — must match tools/build-warforged-dbc.py and the 3.3.5a
# client's hard-coded expectation, or the client rejects the file on Data load.
FIELD_COUNT = 38
RECORD_SIZE = 152  # 38 * 4
EXPECTED_CSV_COLUMNS = 14
EXPECTED_ROW_COUNT = 21

# Field offsets within a 38-uint32 record (verified against the AC struct, copied
# from tools/build-warforged-dbc.py).
F_ID = 0
F_CHARGES = 1
F_EFFECT_BASE = 2          # 2..4
F_EFFECT_MIN_BASE = 5      # 5..7
F_EFFECT_MAX_BASE = 8      # 8..10
F_EFFECT_ARG_BASE = 11     # 11..13
F_NAME_LANG_BASE = 14      # 14..29 (16 locales)
F_NAME_LANG_FLAGS = 30
F_ITEM_VISUAL_ID = 31
F_FLAGS = 32
F_SRC_ITEM_ID = 33
F_CONDITION_ID = 34
F_REQUIRED_SKILL_ID = 35
F_REQUIRED_SKILL_RANK = 36
F_REQUIRED_LEVEL = 37

ENCHANT_NAME = "Warforged"
NAME_LANG_FLAGS = 0xFFFFFFFF  # all-locales-set, matches the server builder


def _parse_csv(csv_path: Path) -> List[dict]:
    """Parse warforged_enchants.csv, skipping comment lines.

    Mirrors tools/build-warforged-dbc.py:parse_csv — the file has a COMMENTED
    header (line starting with '#'), so every '#' line is dropped and the
    remaining lines are pure data. Each row must have exactly 14 columns.
    """
    rows: List[dict] = []
    with Path(csv_path).open(newline="", encoding="utf-8") as f:
        reader = csv.reader(
            line for line in f if line.strip() and not line.lstrip().startswith("#")
        )
        for parts in reader:
            if len(parts) != EXPECTED_CSV_COLUMNS:
                raise ValueError(
                    f"expected {EXPECTED_CSV_COLUMNS} columns, got {len(parts)}: {parts}"
                )
            rows.append({
                "id": int(parts[0]),
                "e1_type": int(parts[5]),
                "e1_amount": int(parts[6]),
                "e1_stat": int(parts[7]),
                "e2_type": int(parts[8]),
                "e2_amount": int(parts[9]),
                "e2_stat": int(parts[10]),
                "e3_type": int(parts[11]),
                "e3_amount": int(parts[12]),
                "e3_stat": int(parts[13]),
            })
    return rows


def _record(row: dict, name_offset: int) -> bytes:
    """Pack one 38-uint32 record. Column->field mapping mirrors the server
    builder (tools/build-warforged-dbc.py:build_record) exactly so the client
    tooltip shows the same enchant stats the server applies."""
    fields = [0] * FIELD_COUNT
    fields[F_ID] = row["id"]
    fields[F_CHARGES] = 0

    fields[F_EFFECT_BASE + 0] = row["e1_type"]
    fields[F_EFFECT_BASE + 1] = row["e2_type"]
    fields[F_EFFECT_BASE + 2] = row["e3_type"]

    fields[F_EFFECT_MIN_BASE + 0] = row["e1_amount"]
    fields[F_EFFECT_MIN_BASE + 1] = row["e2_amount"]
    fields[F_EFFECT_MIN_BASE + 2] = row["e3_amount"]

    # For fixed-value stat enchants, max == min (matches the server builder).
    fields[F_EFFECT_MAX_BASE + 0] = row["e1_amount"]
    fields[F_EFFECT_MAX_BASE + 1] = row["e2_amount"]
    fields[F_EFFECT_MAX_BASE + 2] = row["e3_amount"]

    fields[F_EFFECT_ARG_BASE + 0] = row["e1_stat"]
    fields[F_EFFECT_ARG_BASE + 1] = row["e2_stat"]
    fields[F_EFFECT_ARG_BASE + 2] = row["e3_stat"]

    for i in range(16):
        fields[F_NAME_LANG_BASE + i] = name_offset

    fields[F_NAME_LANG_FLAGS] = NAME_LANG_FLAGS
    # Remaining fields (31..37) stay 0.

    return struct.pack(f"<{FIELD_COUNT}I", *fields)


def build(sources: Dict[str, Path]) -> Dict[str, bytes]:
    """Compose the partial SpellItemEnchantment.dbc (21 ToT rows only).

    Returns {"SpellItemEnchantment.dbc": <bytes>} — the patch-local override
    block the compositor packs into the unified patch MPQ.
    """
    rows = _parse_csv(sources["enchants_csv"])
    if len(rows) != EXPECTED_ROW_COUNT:
        raise ValueError(f"expected {EXPECTED_ROW_COUNT} enchant rows, got {len(rows)}")

    sblock = StringBlock()
    # One shared "Warforged" string; all 21 rows / 16 locale slots point at it.
    name_offset = sblock.insert(ENCHANT_NAME)

    record_buf = bytearray()
    for row in rows:
        record_buf += _record(row, name_offset)

    string_block = sblock.bytes()
    header = pack_header(
        record_count=len(rows),
        field_count=FIELD_COUNT,
        record_size=RECORD_SIZE,
        string_block_size=len(string_block),
    )
    return {"SpellItemEnchantment.dbc": bytes(header) + bytes(record_buf) + string_block}
