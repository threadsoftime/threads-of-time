# tools/dbc-patch-builder/src/dbc_patch_builder/spell_dbc.py
"""Build the Spell.dbc override block for the Bracket 1 patch.

A patch MPQ's Spell.dbc only needs to contain the OVERRIDDEN rows (54). The
client merges per-ID: for any spell ID in the patch, the patch record takes
precedence. Spell IDs absent from the patch fall through to the stock DBC.

3.3.5a Spell.dbc record layout (empirically verified):
- 234 fields per record
- 936 bytes per record (4 bytes per field, all fields uint32/float/string-offset)
- SpellName_Lang_enUS string-offset lives at byte 544 within each record
- SpellDescription_Lang_enUS string-offset lives at byte 680

For overrides we keep all 936 bytes from the baseline EXCEPT the two
string-offset fields, which we replace with offsets into our own
patch-local string block.
"""
from __future__ import annotations
import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List

from .dbc_io import StringBlock, pack_header

SPELL_RECORD_SIZE = 936
SPELL_FIELD_COUNT = 234


@dataclass(frozen=True)
class BaselineRow:
    spell_id: int
    raw_record: bytes        # 936 bytes; name+desc offset bytes will be replaced
    name_offset_byte: int    # byte index within raw_record where name string offset lives
    desc_offset_byte: int    # byte index within raw_record where description string offset lives


@dataclass(frozen=True)
class SpellOverride:
    spell_id: int
    name: str
    description: str


def load_baseline(tsv_path: Path) -> Dict[int, BaselineRow]:
    """TSV format: spell_id<TAB>raw_hex<TAB>name_offset_byte<TAB>desc_offset_byte

    Lines starting with `#` are comments.
    """
    out: Dict[int, BaselineRow] = {}
    for line in Path(tsv_path).read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        spell_id, raw_hex, name_off, desc_off = line.split("\t")
        raw = bytes.fromhex(raw_hex)
        if len(raw) != SPELL_RECORD_SIZE:
            raise ValueError(f"spell_id {spell_id}: raw record is {len(raw)} bytes, expected {SPELL_RECORD_SIZE}")
        out[int(spell_id)] = BaselineRow(
            spell_id=int(spell_id),
            raw_record=raw,
            name_offset_byte=int(name_off),
            desc_offset_byte=int(desc_off),
        )
    return out


def build_spell_dbc_overrides(baseline: Dict[int, BaselineRow], overrides: List[SpellOverride]) -> bytes:
    sblock = StringBlock()
    record_buf = bytearray()

    for ov in overrides:
        base = baseline[ov.spell_id]
        name_off = sblock.insert(ov.name)
        desc_off = sblock.insert(ov.description)

        rec = bytearray(base.raw_record)
        rec[base.name_offset_byte:base.name_offset_byte + 4] = struct.pack("<I", name_off)
        rec[base.desc_offset_byte:base.desc_offset_byte + 4] = struct.pack("<I", desc_off)

        assert len(rec) == SPELL_RECORD_SIZE
        record_buf += rec

    string_block = sblock.bytes()
    header = pack_header(
        record_count=len(overrides),
        field_count=SPELL_FIELD_COUNT,
        record_size=SPELL_RECORD_SIZE,
        string_block_size=len(string_block),
    )
    return bytes(header) + bytes(record_buf) + string_block
