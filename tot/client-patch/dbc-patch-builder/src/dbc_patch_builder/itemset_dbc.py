# tools/dbc-patch-builder/src/dbc_patch_builder/itemset_dbc.py
"""Build ItemSet.dbc for the Bracket 1 patch.

Row layout (3.3.5a, 53 fields, 212 bytes per row):
- ID (uint32)
- Name_Lang (LocString: 16 locale string-offsets + 1 flag mask) = 17 fields
- ItemID_1..17 (17 x uint32)
- SetSpellID_1..8 (8 x uint32)
- SetThreshold_1..8 (8 x uint32)
- RequiredSkill (uint32)
- RequiredSkillRank (uint32)

The 3.3.5a client validates `field_count` in the WDBC header against its hard-coded
expectation. Shipping fewer fields crashes the client with "wrong number of columns"
on Data load. The 16-locale layout has 8 in-use slots (enUS through ruRU) and 8
reserved/padding slots — they must all be present even if zeroed.
"""
from __future__ import annotations
import struct
from dataclasses import dataclass
from typing import List, Sequence, Tuple

from .dbc_io import StringBlock, pack_header

FIELD_COUNT = 53
RECORD_SIZE = 212
LOCALE_SLOTS = 16
LANG_MASK_ALL_LOCALES = 0xFF01FE  # standard lang-mask for 3.3.5a DBC (carried in live client DBCs; client uses positional indexing for display)


@dataclass(frozen=True)
class ItemSetRow:
    id: int
    name: str
    set_spell_ids: Tuple[int, int, int, int, int, int, int, int]
    set_thresholds: Tuple[int, int, int, int, int, int, int, int]
    item_ids: Sequence[int] = (0,) * 17
    required_skill: int = 0
    required_skill_rank: int = 0


def build_itemset_dbc(rows: List[ItemSetRow]) -> bytes:
    sblock = StringBlock()
    record_buf = bytearray()

    for row in rows:
        name_off = sblock.insert(row.name)
        # ID
        record_buf += struct.pack("<I", row.id)
        # 16 locale name offsets (3.3.5a LocString shape: 8 in-use + 8 reserved/padding).
        # enUS is slot 0; the other slots are 0 (client falls back via the lang mask).
        record_buf += struct.pack("<I", name_off)
        record_buf += b"\x00" * (4 * (LOCALE_SLOTS - 1))
        # Lang mask
        record_buf += struct.pack("<I", LANG_MASK_ALL_LOCALES)
        # 17 item IDs
        assert len(row.item_ids) == 17, f"item_ids must be 17 entries, got {len(row.item_ids)}"
        for iid in row.item_ids:
            record_buf += struct.pack("<I", iid)
        # 8 set spell IDs
        for sid in row.set_spell_ids:
            record_buf += struct.pack("<I", sid)
        # 8 set thresholds
        for thr in row.set_thresholds:
            record_buf += struct.pack("<I", thr)
        # Required skill + rank
        record_buf += struct.pack("<II", row.required_skill, row.required_skill_rank)

    string_block = sblock.bytes()
    header = pack_header(
        record_count=len(rows),
        field_count=FIELD_COUNT,
        record_size=RECORD_SIZE,
        string_block_size=len(string_block),
    )
    return bytes(header) + bytes(record_buf) + string_block


def _pad_items(items: Sequence[int]) -> Sequence[int]:
    """Pad/truncate to exactly 17 entries for ItemSet.dbc itemId field."""
    items = list(items)[:17]
    return tuple(items + [0] * (17 - len(items)))


def compose_27_class_spec_rows(bonus_rows, itemset_map, class_items=None) -> List[ItemSetRow]:
    """Compose the 27 per-class+spec ItemSet rows from the bonus map + itemset map.

    bonus_rows: List[BonusRow] (from bonus_map_parser)
    itemset_map: Dict[(class_id, spec_id), itemset_id]  (from bracket_set_itemset_map seed)
    class_items: Optional Dict[class_id, List[int]] — member item entries for each class's
        set. If provided, used to populate itemId[17] so the client renders a valid (N/M)
        denominator and (most importantly) the (2)/(4) Set bonus lines. Without this the
        3.3.5a client treats the set as malformed and silently skips bonus rendering even
        when setSpellID/setThreshold are populated.
    """
    by_cs = {}
    for r in bonus_rows:
        by_cs.setdefault((r.class_id, r.spec_id), {})[r.threshold] = r.spell_id

    rows = []
    for (cls, spec), itemset_id in sorted(itemset_map.items()):
        spells = by_cs.get((cls, spec), {})
        spell_2 = spells.get(2, 0)
        spell_4 = spells.get(4, 0)
        items = class_items.get(cls, []) if class_items else []
        rows.append(ItemSetRow(
            id=itemset_id,
            name="Vestiges of Blackfathom",
            set_spell_ids=(spell_2, spell_4, 0, 0, 0, 0, 0, 0),
            set_thresholds=(2, 4, 0, 0, 0, 0, 0, 0),
            item_ids=_pad_items(items),
        ))
    return rows


def fallback_row() -> ItemSetRow:
    """The 90101 name-only fallback row (edge case §7.2)."""
    return ItemSetRow(
        id=90101,
        name="Vestiges of Blackfathom",
        set_spell_ids=(0,) * 8,
        set_thresholds=(0,) * 8,
    )
