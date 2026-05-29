# tools/dbc-patch-builder/tests/test_spell_dbc.py
import struct
from pathlib import Path
from dbc_patch_builder.spell_dbc import (
    SpellOverride,
    load_baseline,
    build_spell_dbc_overrides,
    SPELL_RECORD_SIZE,
    SPELL_FIELD_COUNT,
)
from dbc_patch_builder.dbc_io import HEADER_SIZE, WDBC_MAGIC

PKG_ROOT = Path(__file__).resolve().parents[1]  # tot/client-patch/dbc-patch-builder/
BASELINE = PKG_ROOT / "baseline" / "spell_dbc_baseline.tsv"


def test_baseline_has_54_rows():
    baseline = load_baseline(BASELINE)
    assert len(baseline) == 54


def test_baseline_records_are_correct_size():
    baseline = load_baseline(BASELINE)
    for spell_id, row in baseline.items():
        assert len(row.raw_record) == SPELL_RECORD_SIZE, f"spell {spell_id}: bad size"


def test_baseline_offset_constants():
    baseline = load_baseline(BASELINE)
    for spell_id, row in baseline.items():
        assert row.name_offset_byte == 544
        assert row.desc_offset_byte == 680


def test_override_writer_produces_valid_wdbc():
    baseline = load_baseline(BASELINE)
    overrides = [
        SpellOverride(spell_id=64938, name="Sundered Resolve",
                      description="Test flavor.\rTest mechanic.\rActive while in Bracket 1 (L25-34)")
    ]
    blob = build_spell_dbc_overrides(baseline, overrides)
    magic, count, fc, rs, _ = struct.unpack("<4sIIII", blob[:HEADER_SIZE])
    assert magic == WDBC_MAGIC
    assert count == 1
    assert fc == SPELL_FIELD_COUNT
    assert rs == SPELL_RECORD_SIZE
    assert b"Sundered Resolve\x00" in blob
    assert b"Test mechanic." in blob


def test_override_preserves_baseline_fields():
    """Confirm that fields OTHER than name+desc are byte-identical to baseline."""
    baseline = load_baseline(BASELINE)
    base = baseline[64938]
    overrides = [SpellOverride(spell_id=64938, name="X", description="Y")]
    blob = build_spell_dbc_overrides(baseline, overrides)
    # Strip header to get the first record
    rec = blob[HEADER_SIZE:HEADER_SIZE + SPELL_RECORD_SIZE]
    # Compare every byte except the 4-byte name field and 4-byte desc field
    for byte_off in range(SPELL_RECORD_SIZE):
        if 544 <= byte_off < 548:
            continue
        if 680 <= byte_off < 684:
            continue
        assert rec[byte_off] == base.raw_record[byte_off], f"divergence at byte {byte_off}"


def test_override_writer_handles_54_rows():
    baseline = load_baseline(BASELINE)
    overrides = [
        SpellOverride(spell_id=sid, name=f"Name {sid}", description=f"Desc {sid}")
        for sid in sorted(baseline.keys())
    ]
    blob = build_spell_dbc_overrides(baseline, overrides)
    _, count, _, _, _ = struct.unpack("<4sIIII", blob[:HEADER_SIZE])
    assert count == 54
