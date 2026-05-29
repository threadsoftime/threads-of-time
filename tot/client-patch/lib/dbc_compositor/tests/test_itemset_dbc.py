# SPDX-License-Identifier: GPL-2.0-or-later
import struct
from dbc_compositor.itemset_dbc import ItemSetRow, build_itemset_dbc
from dbc_compositor.dbc_io import HEADER_SIZE, WDBC_MAGIC


def make_warrior_arms_row():
    return ItemSetRow(
        id=90111,
        name="Vestiges of Blackfathom",
        set_spell_ids=(64938, 64939, 0, 0, 0, 0, 0, 0),
        set_thresholds=(2, 4, 0, 0, 0, 0, 0, 0),
    )


def test_single_row_dbc_has_correct_header():
    blob = build_itemset_dbc([make_warrior_arms_row()])
    magic, count, field_count, record_size, sbs = struct.unpack("<4sIIII", blob[:HEADER_SIZE])
    assert magic == WDBC_MAGIC
    assert count == 1
    assert field_count == 53
    assert record_size == 212


def test_dbc_contains_set_name_in_string_block():
    blob = build_itemset_dbc([make_warrior_arms_row()])
    assert b"Vestiges of Blackfathom\x00" in blob


def test_28_row_dbc_byte_count():
    # Header + 28 records of 180 bytes + string block with deduped "Vestiges of Blackfathom"
    rows = [
        ItemSetRow(id=90101, name="Vestiges of Blackfathom",
                   set_spell_ids=(0,0,0,0,0,0,0,0), set_thresholds=(0,0,0,0,0,0,0,0)),
    ]
    for cls_slot, cls_id in enumerate([1,2,3,4,5,7,8,9,11], start=1):
        for spec in (1,2,3):
            rows.append(ItemSetRow(
                id=90100 + cls_slot * 10 + spec,
                name="Vestiges of Blackfathom",
                set_spell_ids=(64938, 64939, 0,0,0,0,0,0),  # placeholder spell IDs OK for sizing test
                set_thresholds=(2, 4, 0,0,0,0,0,0),
            ))
    blob = build_itemset_dbc(rows)
    magic, count, _, record_size, sbs = struct.unpack("<4sIIII", blob[:HEADER_SIZE])
    assert count == 28
    assert len(blob) == HEADER_SIZE + 28 * record_size + sbs


def test_string_dedup():
    # All 28 rows share the same name -> string block has it exactly once
    rows = [
        ItemSetRow(id=90100 + i, name="Vestiges of Blackfathom",
                   set_spell_ids=(0,)*8, set_thresholds=(0,)*8)
        for i in range(28)
    ]
    blob = build_itemset_dbc(rows)
    assert blob.count(b"Vestiges of Blackfathom\x00") == 1
