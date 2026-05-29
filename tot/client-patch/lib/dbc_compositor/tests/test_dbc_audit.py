# SPDX-License-Identifier: GPL-2.0-or-later
import struct

import pytest

from dbc_compositor.dbc_audit import record_ids, assert_ids_in_ranges, StockRowError


def _wdbc(ids, field_count=2, record_size=8):
    """Build a minimal valid WDBC blob whose first field of each record is the ID."""
    header = struct.pack("<4sIIII", b"WDBC", len(ids), field_count, record_size, 1)
    recs = b"".join(struct.pack("<II", i, 0) for i in ids)
    return header + recs + b"\x00"


def test_record_ids_reads_first_field():
    blob = _wdbc([70001, 70002, 70063])
    assert record_ids(blob) == [70001, 70002, 70063]


def test_record_ids_rejects_non_wdbc():
    with pytest.raises(ValueError, match="WDBC"):
        record_ids(b"NOPExxxxxxxxxxxxxxxxxxxx")


def test_assert_ids_in_ranges_passes_when_contained():
    blob = _wdbc([70001, 70030, 70063])
    assert_ids_in_ranges("SpellItemEnchantment.dbc", blob, [(70001, 70063)], "mod-warforged")


def test_assert_ids_in_ranges_rejects_stock_row():
    """A stock Blizzard row (ID 5) outside the declared ToT range must fail loud."""
    blob = _wdbc([5, 70001])
    with pytest.raises(StockRowError, match="70"):
        assert_ids_in_ranges("SpellItemEnchantment.dbc", blob, [(70001, 70063)], "mod-warforged")


def test_assert_ids_in_ranges_multi_cluster():
    blob = _wdbc([64854, 67200, 70841])
    assert_ids_in_ranges(
        "Spell.dbc", blob, [(64854, 64939), (67121, 67268), (70724, 70841)], "mod-bracket-sets"
    )
