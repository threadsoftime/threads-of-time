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
        assert tpl.params, f"{name}: at least one param required"


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
