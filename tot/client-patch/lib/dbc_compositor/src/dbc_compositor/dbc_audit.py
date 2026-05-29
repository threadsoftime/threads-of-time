# SPDX-License-Identifier: GPL-2.0-or-later
"""Read record IDs from an encoded WDBC blob — collision check B + §10.3 gate.

Every DBC the compositor packs must contain ONLY ToT-original rows whose IDs
fall inside the producing module's declared [id_ranges]. This is both the
row-level collision guarantee and the legal gate that no stock Blizzard row
(e.g. a stock SpellItemEnchantment row with ID < 70001) ships in the MPQ.
"""
from __future__ import annotations

import struct

HEADER_SIZE = 20


class StockRowError(Exception):
    """Raised when a packed DBC contains a row ID outside the declared ranges."""


def record_ids(blob: bytes) -> list[int]:
    """Return the first-field (ID) of every record in a WDBC blob."""
    magic, rec_count, field_count, rec_size, _sb = struct.unpack("<4sIIII", blob[:HEADER_SIZE])
    if magic != b"WDBC":
        raise ValueError(f"not a WDBC blob (magic={magic!r})")
    ids = []
    for i in range(rec_count):
        off = HEADER_SIZE + i * rec_size
        ids.append(struct.unpack_from("<I", blob, off)[0])
    return ids


def assert_ids_in_ranges(
    dbc_name: str, blob: bytes, ranges: list[tuple[int, int]], mod: str
) -> list[int]:
    ids = record_ids(blob)
    for rid in ids:
        if not any(lo <= rid <= hi for (lo, hi) in ranges):
            raise StockRowError(
                f"§10.3 / collision violation: {mod} {dbc_name} contains row ID "
                f"{rid} outside its declared ranges {ranges}. Stock Blizzard rows "
                f"and undeclared IDs must not ship in the client MPQ."
            )
    return ids
