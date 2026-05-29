# SPDX-License-Identifier: GPL-2.0-or-later
"""§10.3: a DBC blob carrying a stock Blizzard row ID must be rejected."""
import struct

import pytest

from dbc_compositor.dbc_audit import assert_ids_in_ranges, StockRowError


def _wdbc(ids):
    header = struct.pack("<4sIIII", b"WDBC", len(ids), 2, 8, 1)
    recs = b"".join(struct.pack("<II", i, 0) for i in ids)
    return header + recs + b"\x00"


def test_warforged_full_file_would_be_rejected():
    """The server-side 2677-row file (stock IDs like 1,2,3) must NOT pass the gate."""
    blob = _wdbc([1, 2, 3, 70001, 70063])  # contains stock rows
    with pytest.raises(StockRowError):
        assert_ids_in_ranges("SpellItemEnchantment.dbc", blob, [(70001, 70063)], "mod-warforged")


def test_warforged_partial_passes():
    blob = _wdbc([70001, 70030, 70063])
    assert_ids_in_ranges("SpellItemEnchantment.dbc", blob, [(70001, 70063)], "mod-warforged")
