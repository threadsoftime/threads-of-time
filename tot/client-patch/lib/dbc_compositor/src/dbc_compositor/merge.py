# SPDX-License-Identifier: GPL-2.0-or-later
"""Merge per-module DBC recipe outputs into one {dbc_name: blob} mapping.

Each recipe returns a COMPLETE encoded DBC blob per filename (the existing
encoders emit whole files, not row deltas). For 1.0.0 no two modules write the
same DBC file: mod-bracket-sets owns ItemSet.dbc + Spell.dbc, mod-warforged
owns SpellItemEnchantment.dbc. So merging is file-level: one producer per DBC
filename. If two modules both emit the same DBC filename, that is a hard error
— row-level union of one DBC file across modules is intentionally out of scope
(would require decode/re-encode; add when a real second producer needs it).

ID-range overlap across modules is guarded by RangeRegistry (manifest.py,
check A); per-file ID containment by dbc_audit (Task 7, check B). This module
is the file-level producer guard.
"""
from __future__ import annotations


class DuplicateProducerError(Exception):
    """Raised when two modules both produce the same DBC filename."""


def merge_dbc_outputs(outputs: dict[str, dict[str, bytes]]) -> dict[str, bytes]:
    """outputs: {mod_name: {dbc_filename: blob}} -> {dbc_filename: blob}."""
    merged: dict[str, bytes] = {}
    producer: dict[str, str] = {}
    for mod, files in outputs.items():
        for dbc_name, blob in files.items():
            if dbc_name in merged:
                raise DuplicateProducerError(
                    f"Two modules produce {dbc_name}: "
                    f"{producer[dbc_name]} and {mod}. "
                    f"Row-level merge of one DBC file across modules is not "
                    f"supported; give them disjoint DBC files or merge upstream."
                )
            merged[dbc_name] = blob
            producer[dbc_name] = mod
    return merged
