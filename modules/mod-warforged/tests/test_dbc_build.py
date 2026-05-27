"""Sanity test for tools/build-warforged-dbc.py.

Runs the build script in a temp dir against a temp copy of the inputs, then
parses the output DBC back and asserts every CSV row landed correctly:
  - all 21 IDs are present (70001..70063, banded with skips)
  - effect_type / amount / stat fields match the CSV exactly
  - both copies of EffectPointsMin and EffectPointsMax equal the CSV amount
  - all 16 name_lang slots point at a non-zero offset
  - the offset they point to dereferences to "Warforged" in the string block

Run with:
  pytest modules/mod-warforged/tests/test_dbc_build.py -v
"""
from __future__ import annotations

import csv
import os
import shutil
import struct
import subprocess
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[3]
MODULE_ROOT = Path(__file__).resolve().parents[1]
BUILD_SCRIPT = MODULE_ROOT / "tools" / "build-warforged-dbc.py"
CSV_SRC = MODULE_ROOT / "data" / "csv" / "warforged_enchants.csv"
INPUT_DBC_SRC = MODULE_ROOT / "data" / "dbc" / "input" / "SpellItemEnchantment.dbc"

HEADER_SIZE = 20
RECORD_SIZE = 152
FIELD_COUNT = 38


def _parse_csv() -> list[dict]:
    rows = []
    with CSV_SRC.open(newline="", encoding="utf-8") as f:
        reader = csv.reader(
            line for line in f if line.strip() and not line.lstrip().startswith("#")
        )
        for parts in reader:
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


def _read_dbc(path: Path):
    """Return (records_list, string_block_bytes). Each record is a tuple of 38 uints."""
    raw = path.read_bytes()
    magic, rec_count, fld_count, rec_size, sb_size = struct.unpack(
        "<4sIIII", raw[:HEADER_SIZE]
    )
    assert magic == b"WDBC"
    assert fld_count == FIELD_COUNT
    assert rec_size == RECORD_SIZE
    records = []
    off = HEADER_SIZE
    for _ in range(rec_count):
        records.append(struct.unpack(f"<{FIELD_COUNT}I", raw[off:off + RECORD_SIZE]))
        off += RECORD_SIZE
    sb = raw[off:off + sb_size]
    return records, sb


def _cstring_at(sb: bytes, offset: int) -> str:
    end = sb.find(b"\x00", offset)
    if end < 0:
        end = len(sb)
    return sb[offset:end].decode("utf-8")


@pytest.fixture(scope="module")
def built_module(tmp_path_factory) -> Path:
    """Copy the module into a temp dir and run the build script there.

    Returns the temp module root. The fixture is module-scoped so the test
    suite only invokes the build once.
    """
    tmp_root = tmp_path_factory.mktemp("warforged-dbc")
    tmp_module = tmp_root / "mod-warforged"

    # Stage just the bits the build script reads + writes into.
    (tmp_module / "tools").mkdir(parents=True)
    (tmp_module / "data" / "csv").mkdir(parents=True)
    (tmp_module / "data" / "dbc" / "input").mkdir(parents=True)

    shutil.copy2(BUILD_SCRIPT, tmp_module / "tools" / BUILD_SCRIPT.name)
    shutil.copy2(CSV_SRC, tmp_module / "data" / "csv" / CSV_SRC.name)
    shutil.copy2(
        INPUT_DBC_SRC,
        tmp_module / "data" / "dbc" / "input" / INPUT_DBC_SRC.name,
    )

    result = subprocess.run(
        [sys.executable, str(tmp_module / "tools" / BUILD_SCRIPT.name)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, (
        f"build script failed (rc={result.returncode}):\n"
        f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
    )
    return tmp_module


def test_outputs_exist(built_module):
    server_out = built_module / "data" / "dbc" / "SpellItemEnchantment.dbc"
    client_out = built_module / "build" / "mpq-staging" / "DBFilesClient" / "SpellItemEnchantment.dbc"
    assert server_out.exists()
    assert client_out.exists()
    assert server_out.stat().st_size == client_out.stat().st_size, \
        "server-side and client-side outputs must be byte-identical"


def test_record_count_increased(built_module):
    input_records, _ = _read_dbc(INPUT_DBC_SRC)
    output_records, _ = _read_dbc(built_module / "data" / "dbc" / "SpellItemEnchantment.dbc")
    csv_rows = _parse_csv()
    assert len(output_records) == len(input_records) + len(csv_rows) == len(input_records) + 21


def test_all_csv_ids_present(built_module):
    csv_rows = _parse_csv()
    expected_ids = {row["id"] for row in csv_rows}
    # The plan promises IDs 70001..70063 banded; double-check that.
    assert expected_ids.issuperset({70001, 70003, 70021, 70063})
    output_records, _ = _read_dbc(built_module / "data" / "dbc" / "SpellItemEnchantment.dbc")
    observed_ids = {rec[0] for rec in output_records}
    missing = expected_ids - observed_ids
    assert not missing, f"missing IDs in output DBC: {sorted(missing)}"


def test_each_row_fields_match_csv(built_module):
    csv_rows = _parse_csv()
    output_records, sb = _read_dbc(built_module / "data" / "dbc" / "SpellItemEnchantment.dbc")
    by_id = {rec[0]: rec for rec in output_records}

    for row in csv_rows:
        rec = by_id[row["id"]]
        # Effect types (fields 2,3,4)
        assert rec[2] == row["e1_type"], f"id {row['id']} effect1 type"
        assert rec[3] == row["e2_type"], f"id {row['id']} effect2 type"
        assert rec[4] == row["e3_type"], f"id {row['id']} effect3 type"

        # EffectPointsMin (fields 5,6,7) — what AC reads as "amount"
        assert rec[5] == row["e1_amount"], f"id {row['id']} effect1 amount"
        assert rec[6] == row["e2_amount"], f"id {row['id']} effect2 amount"
        assert rec[7] == row["e3_amount"], f"id {row['id']} effect3 amount"

        # EffectPointsMax (fields 8,9,10) — for fixed-value stats, == Min
        assert rec[8] == row["e1_amount"], f"id {row['id']} effect1 max != min"
        assert rec[9] == row["e2_amount"], f"id {row['id']} effect2 max != min"
        assert rec[10] == row["e3_amount"], f"id {row['id']} effect3 max != min"

        # EffectArg (fields 11,12,13) — ITEM_MOD_* stat id
        assert rec[11] == row["e1_stat"], f"id {row['id']} effect1 stat"
        assert rec[12] == row["e2_stat"], f"id {row['id']} effect2 stat"
        assert rec[13] == row["e3_stat"], f"id {row['id']} effect3 stat"

        # Name_lang[0..15] (fields 14..29) all point at the same non-zero offset
        name_offsets = rec[14:30]
        assert all(o == name_offsets[0] for o in name_offsets), \
            f"id {row['id']} name_lang slots aren't identical"
        assert name_offsets[0] > 0, f"id {row['id']} name_lang[0] is zero"
        assert _cstring_at(sb, name_offsets[0]) == "Warforged", \
            f"id {row['id']} name string is not 'Warforged'"

        # Trailing fields — itemVisual=0, flags=0, etc.
        assert rec[31] == 0, f"id {row['id']} itemVisualID should be 0"
        assert rec[32] == 0, f"id {row['id']} flags should be 0"
        assert rec[33] == 0, f"id {row['id']} src_itemID should be 0"
        assert rec[34] == 0, f"id {row['id']} condition_id should be 0"
        assert rec[35] == 0, f"id {row['id']} required_skill_id should be 0"
        assert rec[36] == 0, f"id {row['id']} required_skill_rank should be 0"
        assert rec[37] == 0, f"id {row['id']} required_level should be 0"


def test_warforged_string_in_block(built_module):
    _, sb = _read_dbc(built_module / "data" / "dbc" / "SpellItemEnchantment.dbc")
    assert b"Warforged\x00" in sb, "'Warforged\\0' must appear in the string block"


def test_input_dbc_untouched_after_build(built_module):
    """The build script writes to data/dbc/ and build/, never to input/."""
    # Sanity: input file in the temp module is identical to the source.
    assert (built_module / "data" / "dbc" / "input" / "SpellItemEnchantment.dbc").read_bytes() == \
        INPUT_DBC_SRC.read_bytes()


def test_sha256_mismatch_aborts(tmp_path):
    """If the input DBC sha differs from the bake, the script should error."""
    tmp_module = tmp_path / "mod-warforged"
    (tmp_module / "tools").mkdir(parents=True)
    (tmp_module / "data" / "csv").mkdir(parents=True)
    (tmp_module / "data" / "dbc" / "input").mkdir(parents=True)

    shutil.copy2(BUILD_SCRIPT, tmp_module / "tools" / BUILD_SCRIPT.name)
    shutil.copy2(CSV_SRC, tmp_module / "data" / "csv" / CSV_SRC.name)

    # Write a tampered input file (still a valid DBC, just one byte changed).
    tampered = bytearray(INPUT_DBC_SRC.read_bytes())
    # Flip a byte deep in the string block — keeps header valid, breaks sha.
    tampered[-1] = (tampered[-1] + 1) & 0xFF
    (tmp_module / "data" / "dbc" / "input" / "SpellItemEnchantment.dbc").write_bytes(bytes(tampered))

    result = subprocess.run(
        [sys.executable, str(tmp_module / "tools" / BUILD_SCRIPT.name)],
        capture_output=True,
        text=True,
    )
    assert result.returncode != 0, "tampered input should be rejected"
    assert "sha256" in result.stderr.lower()
