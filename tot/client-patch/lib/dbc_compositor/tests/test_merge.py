# SPDX-License-Identifier: GPL-2.0-or-later
import pytest

from dbc_compositor.merge import merge_dbc_outputs, DuplicateProducerError


def test_single_producer_passes_through():
    outputs = {"mod-a": {"Spell.dbc": b"AAA", "ItemSet.dbc": b"BBB"}}
    merged = merge_dbc_outputs(outputs)
    assert merged == {"Spell.dbc": b"AAA", "ItemSet.dbc": b"BBB"}


def test_two_producers_different_files_merge():
    """The real 1.0.0 case: bracket-sets + warforged on different files."""
    outputs = {
        "mod-bracket-sets": {"Spell.dbc": b"AAA", "ItemSet.dbc": b"BBB"},
        "mod-warforged": {"SpellItemEnchantment.dbc": b"CCC"},
    }
    merged = merge_dbc_outputs(outputs)
    assert merged == {"Spell.dbc": b"AAA", "ItemSet.dbc": b"BBB", "SpellItemEnchantment.dbc": b"CCC"}


def test_two_producers_same_file_raises():
    outputs = {"mod-a": {"Spell.dbc": b"AAA"}, "mod-b": {"Spell.dbc": b"CCC"}}
    with pytest.raises(DuplicateProducerError, match="Spell.dbc"):
        merge_dbc_outputs(outputs)


def test_duplicate_message_names_both_mods():
    outputs = {"mod-a": {"Spell.dbc": b"A"}, "mod-b": {"Spell.dbc": b"B"}}
    with pytest.raises(DuplicateProducerError) as exc:
        merge_dbc_outputs(outputs)
    assert "mod-a" in str(exc.value) and "mod-b" in str(exc.value)
