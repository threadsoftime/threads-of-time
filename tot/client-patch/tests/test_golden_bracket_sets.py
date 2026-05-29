# SPDX-License-Identifier: GPL-2.0-or-later
"""The non-negotiable guard: the compositor's bracket-sets DBC output stays
byte-identical to the live-shipped blobs (kb_99501ac5)."""
import importlib.util
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
BRACKET = REPO / "modules" / "mod-bracket-sets" / "client"

from dbc_compositor.manifest import load_manifest  # noqa: E402


def _recipe():
    spec = importlib.util.spec_from_file_location("bs_recipe", BRACKET / "build_dbc.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_compositor_dbc_matches_golden():
    m = load_manifest(BRACKET / "MANIFEST.toml")
    out = _recipe().build(m.sources)
    assert out["ItemSet.dbc"] == (BRACKET / "golden" / "itemset.dbc.golden").read_bytes()
    assert out["Spell.dbc"] == (BRACKET / "golden" / "spell.dbc.golden").read_bytes()
