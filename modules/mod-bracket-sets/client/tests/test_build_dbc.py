# SPDX-License-Identifier: GPL-2.0-or-later
"""Golden regression tests for the mod-bracket-sets client DBC recipe.

These tests lock the recipe output byte-for-byte against golden blobs captured
from the legacy dbc-patch-builder (itemset sha=2a5685d6, spell sha=476cedea).
A change to the compose logic, the item tables, or the source data WILL break
these — that is the intended regression anchor for the whole MPQ compositor.

PATH RESOLUTION (verified empirically — do NOT trust the plan's parents[N] note):
  This file: modules/mod-bracket-sets/client/tests/test_build_dbc.py
    parents[0] = .../client/tests
    parents[1] = .../client                <- recipe (build_dbc.py) lives here
    parents[2] = .../mod-bracket-sets       <- module root
    parents[3] = .../modules
    parents[4] = <repo root>                <- threads-of-time
  Lib src: <repo>/tot/client-patch/lib/dbc_compositor/src  (= parents[4]/tot/...)
Both sys.path additions are asserted below so a wrong depth fails loudly with a
clear message instead of a confusing ImportError.
"""
from __future__ import annotations

import hashlib
import sys
import tomllib
from pathlib import Path

_HERE = Path(__file__).resolve()
CLIENT_DIR = _HERE.parents[1]          # modules/mod-bracket-sets/client (recipe lives here)
REPO_ROOT = _HERE.parents[4]           # threads-of-time
LIB_SRC = REPO_ROOT / "tot" / "client-patch" / "lib" / "dbc_compositor" / "src"

# Fail loudly if the depth math is wrong, BEFORE the import that would otherwise
# raise a confusing ImportError / ModuleNotFoundError.
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

import build_dbc  # noqa: E402  (recipe under modules/mod-bracket-sets/client/)
from dbc_compositor.manifest import load_manifest  # noqa: E402

MANIFEST = CLIENT_DIR / "MANIFEST.toml"
GOLDEN_DIR = CLIENT_DIR / "golden"
ITEMSET_GOLDEN = GOLDEN_DIR / "itemset.dbc.golden"
SPELL_GOLDEN = GOLDEN_DIR / "spell.dbc.golden"


def _run_build() -> dict[str, bytes]:
    manifest = load_manifest(MANIFEST)
    return build_dbc.build(manifest.sources)


def test_build_returns_both_dbcs():
    out = _run_build()
    assert set(out.keys()) == {"ItemSet.dbc", "Spell.dbc"}
    assert isinstance(out["ItemSet.dbc"], (bytes, bytearray))
    assert isinstance(out["Spell.dbc"], (bytes, bytearray))


def test_itemset_matches_golden():
    out = _run_build()
    golden = ITEMSET_GOLDEN.read_bytes()
    assert bytes(out["ItemSet.dbc"]) == golden, (
        f"ItemSet.dbc diverged from golden: "
        f"got {len(out['ItemSet.dbc'])}B sha={hashlib.sha256(bytes(out['ItemSet.dbc'])).hexdigest()[:16]}, "
        f"want {len(golden)}B sha={hashlib.sha256(golden).hexdigest()[:16]}"
    )


def test_spell_matches_golden():
    out = _run_build()
    golden = SPELL_GOLDEN.read_bytes()
    assert bytes(out["Spell.dbc"]) == golden, (
        f"Spell.dbc diverged from golden: "
        f"got {len(out['Spell.dbc'])}B sha={hashlib.sha256(bytes(out['Spell.dbc'])).hexdigest()[:16]}, "
        f"want {len(golden)}B sha={hashlib.sha256(golden).hexdigest()[:16]}"
    )


def test_build_is_deterministic():
    a = _run_build()
    b = _run_build()
    for name in ("ItemSet.dbc", "Spell.dbc"):
        assert bytes(a[name]) == bytes(b[name]), f"{name} non-deterministic across runs"


def test_all_54_spell_ids_within_declared_ranges():
    """Cross-check: every authored Spell override ID falls inside the MANIFEST
    [id_ranges]['Spell.dbc'] clusters (no out-of-range leakage)."""
    manifest = load_manifest(MANIFEST)
    ranges = manifest.id_ranges["Spell.dbc"]

    # Pull the 54 authored spell IDs straight from the source seed.
    with open(MANIFEST, "rb") as f:
        tomllib.load(f)  # sanity: manifest parses
    from dbc_compositor.bonus_map_parser import parse_bonus_map_seed
    bonus_rows = parse_bonus_map_seed(manifest.sources["bonus_map"])
    spell_ids = sorted(r.spell_id for r in bonus_rows)
    assert len(spell_ids) == 54, f"expected 54 spell IDs, got {len(spell_ids)}"

    for sid in spell_ids:
        assert any(lo <= sid <= hi for (lo, hi) in ranges), (
            f"spell {sid} outside declared Spell.dbc ranges {ranges}"
        )
