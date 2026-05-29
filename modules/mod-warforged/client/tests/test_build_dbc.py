# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for the mod-warforged client DBC recipe (§10.3-clean partial DBC).

The client ships ONLY the 21 ToT Warforged enchant rows (IDs 70001..70063) in
SpellItemEnchantment.dbc — NEVER the 2656 stock Blizzard rows that the
server-side full-file builder (tools/build-warforged-dbc.py) produces. These
tests lock that contract: exactly 21 rows, all in-range, correct WDBC schema,
deterministic output.

PATH RESOLUTION (verified empirically against the Task 3 sibling test — do NOT
trust the plan's parents[3] note, which overshoots by one to the repo's PARENT):
  This file: modules/mod-warforged/client/tests/test_build_dbc.py
    parents[0] = .../client/tests
    parents[1] = .../client                <- recipe (build_dbc.py) lives here
    parents[2] = .../mod-warforged          <- module root
    parents[3] = .../modules
    parents[4] = <repo root>                <- threads-of-time
  Lib src: <repo>/tot/client-patch/lib/dbc_compositor/src  (= parents[4]/tot/...)
Both sys.path additions are asserted below so a wrong depth fails loudly with a
clear message instead of a confusing ImportError.
"""
from __future__ import annotations

import struct
import sys
from pathlib import Path

_HERE = Path(__file__).resolve()
CLIENT_DIR = _HERE.parents[1]          # modules/mod-warforged/client (recipe lives here)
REPO_ROOT = _HERE.parents[4]           # threads-of-time
LIB_SRC = REPO_ROOT / "tot" / "client-patch" / "lib" / "dbc_compositor" / "src"

# Print + fail loudly if the depth math is wrong, BEFORE the import that would
# otherwise raise a confusing ImportError / ModuleNotFoundError. (Reality-check 1:
# the off-by-one that bit Task 3 — CLIENT_DIR.parents[3] overshoots to the repo's
# PARENT; the correct lib root is reachable via _HERE.parents[4].)
print(f"[path] CLIENT_DIR = {CLIENT_DIR}", file=sys.stderr)
print(f"[path] LIB_SRC    = {LIB_SRC}", file=sys.stderr)
assert (CLIENT_DIR / "build_dbc.py").is_file(), (
    f"recipe build_dbc.py not found at {CLIENT_DIR} — CLIENT_DIR (parents[1]) is wrong"
)
assert (LIB_SRC / "dbc_compositor").is_dir(), (
    f"dbc_compositor package not found under {LIB_SRC} — "
    f"LIB_SRC (parents[4]/tot/client-patch/lib/dbc_compositor/src) is wrong"
)

for p in (str(CLIENT_DIR), str(LIB_SRC)):
    if p not in sys.path:
        sys.path.insert(0, p)

import build_dbc  # noqa: E402  (recipe under modules/mod-warforged/client/)
from dbc_compositor.dbc_audit import record_ids  # noqa: E402
from dbc_compositor.manifest import load_manifest  # noqa: E402

MANIFEST = CLIENT_DIR / "MANIFEST.toml"

HEADER_SIZE = 20
EXPECTED_FIELD_COUNT = 38
EXPECTED_RECORD_SIZE = 152  # 38 * 4
EXPECTED_ROW_COUNT = 21
RANGE_MIN = 70001
RANGE_MAX = 70063


def _run_build() -> dict[str, bytes]:
    manifest = load_manifest(MANIFEST)
    return build_dbc.build(manifest.sources)


def test_build_returns_only_spell_item_enchantment():
    out = _run_build()
    assert set(out.keys()) == {"SpellItemEnchantment.dbc"}
    assert isinstance(out["SpellItemEnchantment.dbc"], (bytes, bytearray))


def test_exactly_21_rows_no_stock():
    """Exactly 21 rows, every ID inside [70001, 70063] — no stock Blizzard rows
    (the full-file server builder produces 2656+21; the client gets ONLY the 21)."""
    out = _run_build()
    ids = record_ids(bytes(out["SpellItemEnchantment.dbc"]))
    assert len(ids) == EXPECTED_ROW_COUNT, f"expected 21 rows, got {len(ids)}: {ids}"
    assert min(ids) >= RANGE_MIN, f"row ID {min(ids)} below {RANGE_MIN} (stock leakage)"
    assert max(ids) <= RANGE_MAX, f"row ID {max(ids)} above {RANGE_MAX} (stock leakage)"


def test_all_ids_within_declared_range():
    """Cross-check every produced ID against the MANIFEST [id_ranges]."""
    manifest = load_manifest(MANIFEST)
    ranges = manifest.id_ranges["SpellItemEnchantment.dbc"]
    out = _run_build()
    ids = record_ids(bytes(out["SpellItemEnchantment.dbc"]))
    for rid in ids:
        assert any(lo <= rid <= hi for (lo, hi) in ranges), (
            f"row ID {rid} outside declared ranges {ranges}"
        )


def test_header_schema():
    """WDBC magic, record_count=21, field_count=38, record_size=152."""
    out = _run_build()
    blob = bytes(out["SpellItemEnchantment.dbc"])
    magic, rec_count, field_count, rec_size, sb_size = struct.unpack(
        "<4sIIII", blob[:HEADER_SIZE]
    )
    assert magic == b"WDBC", f"bad magic {magic!r}"
    assert rec_count == EXPECTED_ROW_COUNT, f"record_count {rec_count} != {EXPECTED_ROW_COUNT}"
    assert field_count == EXPECTED_FIELD_COUNT, f"field_count {field_count} != {EXPECTED_FIELD_COUNT}"
    assert rec_size == EXPECTED_RECORD_SIZE, f"record_size {rec_size} != {EXPECTED_RECORD_SIZE}"
    # File size = header + records + string block.
    assert len(blob) == HEADER_SIZE + rec_count * rec_size + sb_size


def test_build_is_deterministic():
    a = _run_build()
    b = _run_build()
    assert bytes(a["SpellItemEnchantment.dbc"]) == bytes(b["SpellItemEnchantment.dbc"]), (
        "SpellItemEnchantment.dbc non-deterministic across runs"
    )
