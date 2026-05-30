"""Tests for the tool registry and per-tool entries."""

from __future__ import annotations

import pytest

from harness_daemon.registry import (
    ToolEntry,
    ToolNotFound,
    SelfBindingViolation,
    build_v1_registry,
)


def test_obs_ping_registered() -> None:
    reg = build_v1_registry()
    entry = reg.find("obs.ping")
    assert entry.name == "obs.ping"
    assert entry.required_scope == "obs.ping"
    assert entry.subject_guid_arg is None
    assert entry.forwards_to_ac is True


def test_gm_additem_registered_with_target_guid_arg() -> None:
    reg = build_v1_registry()
    entry = reg.find("gm.additem")
    assert entry.subject_guid_arg == "target_guid"


def test_bot_set_goal_uses_bot_guid() -> None:
    reg = build_v1_registry()
    entry = reg.find("bot.set_goal")
    assert entry.subject_guid_arg == "bot_guid"


def test_unknown_tool_raises() -> None:
    reg = build_v1_registry()
    with pytest.raises(ToolNotFound):
        reg.find("gm.nope")


def test_self_binding_check_passes_when_match() -> None:
    reg = build_v1_registry()
    entry = reg.find("gm.additem")
    # GM tools have subject_guid_arg='target_guid' but self-binding
    # only enforced when scope pattern is *.self.*; the registry
    # provides the helper but callers decide when to invoke it.
    entry.check_self_binding({"target_guid": 12345}, bound_to_guid=12345)


def test_self_binding_check_rejects_mismatch() -> None:
    reg = build_v1_registry()
    entry = reg.find("gm.additem")
    with pytest.raises(SelfBindingViolation, match="not_bound"):
        entry.check_self_binding({"target_guid": 99}, bound_to_guid=12345)


def test_self_binding_check_rejects_missing_subject() -> None:
    reg = build_v1_registry()
    entry = reg.find("gm.additem")
    with pytest.raises(SelfBindingViolation, match="missing"):
        entry.check_self_binding({}, bound_to_guid=12345)


def test_obs_query_db_is_daemon_direct() -> None:
    reg = build_v1_registry()
    entry = reg.find("obs.query_db")
    assert entry.forwards_to_ac is False
    assert entry.required_scope == "obs.query_db"


def test_obs_game_events_registered() -> None:
    reg = build_v1_registry()
    entry = reg.find("obs.game_events")
    assert entry.name == "obs.game_events"
    assert entry.required_scope == "obs.game_events"
    assert entry.subject_guid_arg is None
    assert entry.forwards_to_ac is True
