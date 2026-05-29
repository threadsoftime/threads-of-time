# SPDX-License-Identifier: GPL-2.0-or-later
"""Shared low-level DBC IO: WDBC header + string-block builder."""
from __future__ import annotations
import struct
from dataclasses import dataclass, field
from typing import Dict


WDBC_MAGIC = b"WDBC"
HEADER_FMT = "<4sIIII"  # magic, record_count, field_count, record_size, string_block_size
HEADER_SIZE = struct.calcsize(HEADER_FMT)


@dataclass
class StringBlock:
    """Deduplicates strings; returns offset for each insert."""
    _data: bytearray = field(default_factory=lambda: bytearray(b"\x00"))  # offset 0 = empty string per WDBC convention
    _offsets: Dict[str, int] = field(default_factory=dict)

    def insert(self, s: str) -> int:
        """Insert `s`, return its offset. Empty string -> 0."""
        if not s:
            return 0
        if s in self._offsets:
            return self._offsets[s]
        offset = len(self._data)
        self._data.extend(s.encode("utf-8"))
        self._data.append(0)
        self._offsets[s] = offset
        return offset

    def bytes(self) -> bytes:
        return bytes(self._data)


def pack_header(record_count: int, field_count: int, record_size: int, string_block_size: int) -> bytes:
    return struct.pack(HEADER_FMT, WDBC_MAGIC, record_count, field_count, record_size, string_block_size)
