"""Contract assertions: brain's hardcoded harness MCP call sites match live schemas.

Each test pins one CALL SITE in brain code to its expected args schema. A red
test points to the brain file that will fail at runtime if the schema drifts.

Tests are async because they consume the async `harness_tools` fixture (which
opens an MCP session in the background). pytest-asyncio's `auto` mode runs
async tests without needing explicit @pytest.mark.asyncio.
"""
from __future__ import annotations

import pytest

from brain_sidecar.schema_builder import unwrap_fastmcp_args


def _required_and_props(tool):
    schema = unwrap_fastmcp_args(tool.inputSchema)
    return set(schema.get("required", [])), schema.get("properties", {})


def _is_integer_compatible(field_schema):
    """Accept any of: {"type": "integer"}, {"type": ["integer", "null"]}, {"anyOf": [{"type": "integer"}, ...]}."""
    if "type" in field_schema:
        t = field_schema["type"]
        if t == "integer":
            return True
        if isinstance(t, list) and "integer" in t:
            return True
    if "anyOf" in field_schema:
        for branch in field_schema["anyOf"]:
            if branch.get("type") == "integer":
                return True
    return False


@pytest.mark.contract
async def test_obs_get_state_call_site(harness_tools):
    # triage.py:83 — self.harness_mcp.call("obs.get_state", {"target_guid": bot_guid})
    tool = harness_tools["obs.get_state"]
    required, props = _required_and_props(tool)
    assert "target_guid" in required, \
        "obs.get_state lost target_guid (triage.py:83 will fail)"
    assert props["target_guid"]["type"] == "integer"


@pytest.mark.contract
async def test_obs_get_combat_log_call_site(harness_tools):
    # triage.py:85 — args = {target_guid, since_ts_ms, limit}
    # NOTE: since_ts_ms and limit are pydantic Optional[int], serialized as
    # {"anyOf": [{"type": "integer"}, {"type": "null"}]} (no top-level "type").
    tool = harness_tools["obs.get_combat_log"]
    required, props = _required_and_props(tool)
    assert "target_guid" in required, \
        "obs.get_combat_log lost target_guid (triage.py:85 will fail)"
    assert props["target_guid"]["type"] == "integer"
    assert "since_ts_ms" in props
    assert _is_integer_compatible(props["since_ts_ms"]), \
        f"since_ts_ms is not integer-compatible: {props['since_ts_ms']}"
    assert "limit" in props
    assert _is_integer_compatible(props["limit"]), \
        f"limit is not integer-compatible: {props['limit']}"


@pytest.mark.contract
async def test_bot_send_chat_call_site(harness_tools):
    # dispatch.py:233 — args = {bot_guid, channel, message, recipient_name?}
    # NOTE: harness uses "message" (not "text") — bug from pre-v0.1.7 dispatch fixed.
    tool = harness_tools["bot.send_chat"]
    required, props = _required_and_props(tool)
    assert "bot_guid" in required, \
        "bot.send_chat lost bot_guid (dispatch.py:233 will fail)"
    assert "channel" in required
    assert "message" in required, \
        "bot.send_chat expects 'message' field — dispatch.py must send 'message', not 'text'"
    # recipient_name is optional (only required for whisper channel)
    assert "recipient_name" in props


@pytest.mark.contract
async def test_bot_follow_call_site(harness_tools):
    # dispatch.py — dynamic via Decision; pinned here so grammar union sees the right schema.
    tool = harness_tools["bot.follow"]
    required, props = _required_and_props(tool)
    assert required == {"bot_guid", "leader_guid"}, \
        f"bot.follow required-set drifted: {required}"
    assert props["bot_guid"]["type"] == "integer"
    assert props["leader_guid"]["type"] == "integer"


@pytest.mark.contract
async def test_bot_stop_call_site(harness_tools):
    tool = harness_tools["bot.stop"]
    required, props = _required_and_props(tool)
    assert "bot_guid" in required, \
        "bot.stop lost bot_guid"
    assert "clear_combat" in props


@pytest.mark.contract
async def test_bot_set_strategy_call_site(harness_tools):
    tool = harness_tools["bot.set_strategy"]
    required, props = _required_and_props(tool)
    assert "bot_guid" in required
    assert "strategy" in required
    assert "bot_state" in props


@pytest.mark.contract
async def test_bot_set_goal_call_site(harness_tools):
    tool = harness_tools["bot.set_goal"]
    required, props = _required_and_props(tool)
    assert "bot_guid" in required
    assert "goal" in required
    # goal is an unconstrained dict[str, Any] per V0.3 schema patch (kb_642162c3).
    assert props["goal"]["type"] == "object"
