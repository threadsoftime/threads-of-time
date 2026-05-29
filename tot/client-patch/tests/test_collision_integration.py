# SPDX-License-Identifier: GPL-2.0-or-later
"""Prove RangeRegistry fires when a real module would overlap. We exercise the
registry with the real bracket-sets + warforged manifests + the fake overlapping
manifest (not by dropping the fixture into modules/, which would pollute a build)."""
from pathlib import Path

import pytest

from dbc_compositor.manifest import load_manifest, RangeRegistry, CollisionError

REPO = Path(__file__).resolve().parents[3]
BRACKET = REPO / "modules" / "mod-bracket-sets" / "client" / "MANIFEST.toml"
WARFORGED = REPO / "modules" / "mod-warforged" / "client" / "MANIFEST.toml"
FAKE = Path(__file__).resolve().parent / "fixtures" / "fakemod" / "client" / "MANIFEST.toml"


def test_real_manifests_load():
    b = load_manifest(BRACKET)
    assert b.id_ranges["Spell.dbc"] == [(64854, 64939), (67121, 67268), (70724, 70841)]
    w = load_manifest(WARFORGED)
    assert w.id_ranges["SpellItemEnchantment.dbc"] == [(70001, 70063)]


def test_two_real_modules_do_not_collide():
    """Different files => no collision even though 70001-70063 ⊂ 70724-70841 numerically."""
    reg = RangeRegistry()
    for mpath in (BRACKET, WARFORGED):
        m = load_manifest(mpath)
        for name, ranges in m.id_ranges.items():
            reg.add(m.mod, name, ranges)  # no raise


def test_overlap_with_bracket_sets_raises():
    reg = RangeRegistry()
    b = load_manifest(BRACKET)
    f = load_manifest(FAKE)
    for name, ranges in b.id_ranges.items():
        reg.add(b.mod, name, ranges)
    with pytest.raises(CollisionError, match="overlap"):
        for name, ranges in f.id_ranges.items():
            reg.add(f.mod, name, ranges)
