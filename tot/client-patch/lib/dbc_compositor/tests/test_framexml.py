# SPDX-License-Identifier: GPL-2.0-or-later
import pytest

from dbc_compositor.framexml import resolve_overrides, FrameXmlConflictError


def test_no_overrides_returns_empty():
    assert resolve_overrides([]) == {}


def test_single_override():
    entries = [{"mod": "mod-a", "src": "/a/X.xml", "dest": "Interface/X.xml", "priority": 10}]
    out = resolve_overrides(entries)
    assert out == {"Interface/X.xml": "/a/X.xml"}


def test_higher_priority_wins_same_dest():
    entries = [
        {"mod": "mod-a", "src": "/a/X.xml", "dest": "Interface/X.xml", "priority": 10},
        {"mod": "mod-b", "src": "/b/X.xml", "dest": "Interface/X.xml", "priority": 20},
    ]
    out = resolve_overrides(entries)
    assert out["Interface/X.xml"] == "/b/X.xml"  # priority 20 wins


def test_equal_priority_same_dest_raises():
    entries = [
        {"mod": "mod-a", "src": "/a/X.xml", "dest": "Interface/X.xml", "priority": 10},
        {"mod": "mod-b", "src": "/b/X.xml", "dest": "Interface/X.xml", "priority": 10},
    ]
    with pytest.raises(FrameXmlConflictError, match="Interface/X.xml"):
        resolve_overrides(entries)


def test_different_dests_coexist():
    entries = [
        {"mod": "mod-a", "src": "/a/X.xml", "dest": "Interface/X.xml", "priority": 10},
        {"mod": "mod-b", "src": "/b/Y.xml", "dest": "Interface/Y.xml", "priority": 10},
    ]
    out = resolve_overrides(entries)
    assert len(out) == 2
