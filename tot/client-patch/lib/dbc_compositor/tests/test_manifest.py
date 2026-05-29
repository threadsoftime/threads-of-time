# SPDX-License-Identifier: GPL-2.0-or-later
import textwrap
from pathlib import Path

import pytest

from dbc_compositor.manifest import (
    Manifest,
    RangeRegistry,
    CollisionError,
    load_manifest,
)


def _write(tmp_path: Path, body: str, mod_root_files=None) -> Path:
    client = tmp_path / "client"
    client.mkdir(parents=True)
    (client / "MANIFEST.toml").write_text(textwrap.dedent(body))
    for rel in (mod_root_files or []):
        f = tmp_path / rel
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text("x")
    return client / "MANIFEST.toml"


def test_load_manifest_parses_fields_and_resolves_sources(tmp_path):
    mpath = _write(
        tmp_path,
        """
        [manifest]
        mod = "mod-example"
        [sources]
        seed = "data/seed.sql"
        [recipe]
        entry = "build_dbc"
        [id_ranges]
        "ItemSet.dbc" = [{ min = 90100, max = 90199 }]
        """,
        mod_root_files=["data/seed.sql"],
    )
    m = load_manifest(mpath)
    assert m.mod == "mod-example"
    assert m.recipe_entry == "build_dbc"
    assert m.sources["seed"] == (tmp_path / "data/seed.sql").resolve()
    assert m.id_ranges["ItemSet.dbc"] == [(90100, 90199)]


def test_load_manifest_multiple_disjoint_ranges(tmp_path):
    mpath = _write(
        tmp_path,
        """
        [manifest]
        mod = "mod-example"
        [recipe]
        entry = "build_dbc"
        [id_ranges]
        "Spell.dbc" = [
            { min = 64854, max = 64939 },
            { min = 67121, max = 67268 },
            { min = 70724, max = 70841 },
        ]
        """,
    )
    m = load_manifest(mpath)
    assert m.id_ranges["Spell.dbc"] == [(64854, 64939), (67121, 67268), (70724, 70841)]


def test_load_manifest_missing_source_file_raises(tmp_path):
    mpath = _write(
        tmp_path,
        """
        [manifest]
        mod = "mod-example"
        [sources]
        seed = "data/missing.sql"
        [recipe]
        entry = "build_dbc"
        [id_ranges]
        "ItemSet.dbc" = [{ min = 1, max = 2 }]
        """,
    )
    with pytest.raises(FileNotFoundError, match="missing.sql"):
        load_manifest(mpath)


def test_range_registry_accepts_disjoint():
    reg = RangeRegistry()
    reg.add("mod-a", "Spell.dbc", [(64854, 64939)])
    reg.add("mod-b", "Spell.dbc", [(99000, 99099)])  # no raise


def test_range_registry_rejects_overlap():
    reg = RangeRegistry()
    reg.add("mod-a", "Spell.dbc", [(64854, 64939)])
    with pytest.raises(CollisionError, match="overlap"):
        reg.add("mod-b", "Spell.dbc", [(64900, 99099)])


def test_range_registry_overlap_message_names_both_modules():
    reg = RangeRegistry()
    reg.add("mod-a", "Spell.dbc", [(10, 20)])
    with pytest.raises(CollisionError) as exc:
        reg.add("mod-b", "Spell.dbc", [(15, 25)])
    msg = str(exc.value)
    assert "mod-a" in msg and "mod-b" in msg and "Spell.dbc" in msg


def test_range_registry_same_ids_different_dbc_no_collision():
    """Per-file: warforged SpellItemEnchantment.dbc 70001-70063 must NOT collide
    with bracket-sets Spell.dbc even though ranges overlap numerically."""
    reg = RangeRegistry()
    reg.add("mod-bracket-sets", "Spell.dbc", [(70724, 70841)])
    reg.add("mod-warforged", "SpellItemEnchantment.dbc", [(70001, 70063)])  # different file -> fine


def test_range_registry_rejects_touching_endpoints():
    """ID 20 is claimed by both modules — touching endpoints are an overlap."""
    reg = RangeRegistry()
    reg.add("mod-a", "Spell.dbc", [(10, 20)])
    with pytest.raises(CollisionError):
        reg.add("mod-b", "Spell.dbc", [(20, 30)])


def test_load_manifest_rejects_inverted_range(tmp_path):
    """A range with min > max must raise ValueError before the registry sees it."""
    mpath = _write(
        tmp_path,
        """
        [manifest]
        mod = "bad-mod"
        [recipe]
        entry = "build_dbc"
        [id_ranges]
        "Spell.dbc" = [{ min = 200, max = 100 }]
        """,
    )
    with pytest.raises(ValueError, match="inverted"):
        load_manifest(mpath)


def test_range_registry_overlap_message_includes_coordinates():
    """Strengthen the overlap message: coordinates must appear for debuggability."""
    reg = RangeRegistry()
    reg.add("mod-a", "Spell.dbc", [(10, 20)])
    with pytest.raises(CollisionError) as exc:
        reg.add("mod-b", "Spell.dbc", [(15, 25)])
    msg = str(exc.value)
    # overlap coords: max(10,15)=15, min(20,25)=20 must both appear
    assert "15" in msg and "20" in msg
