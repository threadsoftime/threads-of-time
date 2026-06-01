"""Unit tests for the DB allowlist (without a real MySQL connection)."""

from __future__ import annotations

import pytest

from harness_daemon.db_client import (
    BadParams,
    UnknownTemplate,
    V1_TEMPLATES,
)


def test_v1_templates_well_formed() -> None:
    for name, tpl in V1_TEMPLATES.items():
        assert tpl.name == name
        assert tpl.sql.strip().startswith("SELECT"), f"{name}: non-SELECT not allowed"
        assert tpl.db in ("acore_world", "acore_characters", "acore_auth"), \
               f"{name}: unexpected db"
        # params may be [] for full-table-scan templates (e.g. game_event_all).
        # All param names must be non-empty strings when present.
        assert isinstance(tpl.params, list), f"{name}: params must be a list"
        for p in tpl.params:
            assert isinstance(p, str) and p, f"{name}: param names must be non-empty strings"


def test_bracket_set_bonus_map_signature() -> None:
    tpl = V1_TEMPLATES["bracket_set_bonus_map_for"]
    assert tpl.db == "acore_world"
    assert tpl.params == ["itemset_id", "class_id", "spec_id"]


def test_no_template_has_string_interpolation() -> None:
    # Defense in depth: no template should use string-format placeholders.
    # All params must be passed via %s (parameterized).
    for name, tpl in V1_TEMPLATES.items():
        assert "{" not in tpl.sql, f"{name}: f-string-style placeholders forbidden"
        assert "format" not in tpl.sql.lower(), f"{name}: SQL .format() forbidden"


def test_game_event_all_registered() -> None:
    """game_event_all must be in V1_TEMPLATES — the Rust GES slice reads it
    for raw DB columns (anti-circularity: do not use C++ resolved Start/End
    as compute inputs).
    """
    assert "game_event_all" in V1_TEMPLATES, (
        "game_event_all template missing from V1_TEMPLATES"
    )


def test_game_event_all_signature() -> None:
    """Shape: world DB, no parameters (full-table scan), SELECT-only."""
    tpl = V1_TEMPLATES["game_event_all"]
    assert tpl.db == "acore_world"
    assert tpl.params == [], "game_event_all is parameter-free"
    assert tpl.sql.strip().upper().startswith("SELECT")
    # Verified schema columns (DESCRIBE acore_world.game_event 2026-05-30):
    # eventEntry, start_time, end_time, occurence, length, holiday,
    # holidayStage, description, world_event, announce
    # Columns state/nextstart do NOT exist in this fork — must not appear.
    assert "eventEntry" in tpl.sql
    assert "start_time" in tpl.sql
    assert "end_time" in tpl.sql
    assert "occurence" in tpl.sql          # AC canonical misspelling
    assert "length" in tpl.sql
    assert "holiday" in tpl.sql
    assert "holidayStage" in tpl.sql
    # Columns that must NOT appear (absent from this fork's schema)
    assert "state" not in tpl.sql.lower(), "state column does not exist in this fork"
    assert "nextstart" not in tpl.sql.lower(), "nextstart column does not exist in this fork"
