"""Unit tests for brain_sidecar.schema_builder."""
from __future__ import annotations

import pytest

from brain_sidecar.schema_builder import unwrap_fastmcp_args


def test_unwrap_fastmcp_args_strips_args_layer():
    raw = {
        "type": "object",
        "properties": {
            "args": {
                "type": "object",
                "properties": {
                    "bot_guid": {"type": "integer"},
                    "leader_guid": {"type": "integer"},
                },
                "required": ["bot_guid", "leader_guid"],
            }
        },
        "required": ["args"],
    }
    inner = unwrap_fastmcp_args(raw)
    assert inner["type"] == "object"
    assert inner["properties"]["bot_guid"]["type"] == "integer"
    assert inner["required"] == ["bot_guid", "leader_guid"]


def test_unwrap_fastmcp_args_raises_on_missing_args():
    with pytest.raises(ValueError, match="missing required 'args'"):
        unwrap_fastmcp_args({"properties": {"foo": {}}, "required": ["foo"]})


def test_unwrap_fastmcp_args_raises_on_missing_properties():
    with pytest.raises(ValueError, match="missing required 'args'"):
        unwrap_fastmcp_args({"type": "object"})


from brain_sidecar.schema_builder import ToolEntry, compose_oneof


def _follow_entry() -> ToolEntry:
    return ToolEntry(
        description="follow another player; durable",
        schema={
            "type": "object",
            "properties": {
                "bot_guid": {"type": "integer"},
                "leader_guid": {"type": "integer"},
            },
            "required": ["bot_guid", "leader_guid"],
        },
        source_mcp="harness",
    )


def test_compose_oneof_includes_no_op_branch():
    union = compose_oneof({"bot.follow": _follow_entry()})
    titles = [b["title"] for b in union["oneOf"]]
    assert "no_op" in titles


def test_compose_oneof_includes_action_branch_per_tool():
    union = compose_oneof({"bot.follow": _follow_entry()})
    titles = [b["title"] for b in union["oneOf"]]
    assert "action:bot.follow" in titles


def test_compose_oneof_action_branch_pins_tool_const_and_args_shape():
    union = compose_oneof({"bot.follow": _follow_entry()})
    action = next(b for b in union["oneOf"] if b["title"] == "action:bot.follow")
    assert action["properties"]["tool"]["const"] == "bot.follow"
    assert action["properties"]["kind"]["const"] == "action"
    assert action["properties"]["args"]["properties"]["bot_guid"]["type"] == "integer"
    assert action["required"] == ["kind", "tool", "args", "confidence", "reasoning"]


def test_compose_oneof_no_op_branch_has_null_tool_and_args():
    union = compose_oneof({})
    no_op = next(b for b in union["oneOf"] if b["title"] == "no_op")
    assert no_op["properties"]["tool"]["type"] == "null"
    assert no_op["properties"]["args"]["type"] == "null"
    assert no_op["properties"]["kind"]["const"] == "no_op"


from brain_sidecar.schema_builder import render_prompt_summary


def test_render_prompt_summary_includes_required_args_with_types():
    per_tool = {"bot.follow": _follow_entry()}
    text = render_prompt_summary(per_tool)
    assert "bot.follow(bot_guid: int, leader_guid: int)" in text


def test_render_prompt_summary_includes_description():
    per_tool = {"bot.follow": _follow_entry()}
    text = render_prompt_summary(per_tool)
    assert "— follow another player; durable" in text


def test_render_prompt_summary_marks_optional_args_with_qmark():
    per_tool = {
        "bot.send_chat": ToolEntry(
            description="speak in a channel",
            schema={
                "type": "object",
                "properties": {
                    "bot_guid": {"type": "integer"},
                    "channel": {"type": "string"},
                    "text": {"type": "string"},
                    "recipient_name": {"type": "string"},
                },
                "required": ["bot_guid", "channel", "text"],
            },
            source_mcp="harness",
        ),
    }
    text = render_prompt_summary(per_tool)
    assert "recipient_name?: str" in text


def test_render_prompt_summary_omits_dash_when_description_empty():
    per_tool = {
        "x.tool": ToolEntry(
            description="",
            schema={"type": "object", "properties": {}, "required": []},
            source_mcp="harness",
        ),
    }
    text = render_prompt_summary(per_tool)
    # The tool name + parens are present; the trailing "— ..." is absent.
    assert "x.tool()" in text
    assert "— " not in text


def test_render_prompt_summary_sorted_alphabetically():
    per_tool = {
        "zzz.last": _follow_entry(),
        "aaa.first": _follow_entry(),
    }
    text = render_prompt_summary(per_tool)
    assert text.index("aaa.first") < text.index("zzz.last")


from unittest.mock import AsyncMock, MagicMock

from brain_sidecar.schema_builder import fetch_schemas


def _mock_tool(name, description, args_schema):
    tool = MagicMock()
    tool.name = name
    tool.description = description
    tool.inputSchema = {
        "type": "object",
        "properties": {"args": args_schema},
        "required": ["args"],
    }
    return tool


def _client_with_tools(tools):
    client = MagicMock()
    client.list_tools = AsyncMock(return_value=tools)
    return client


@pytest.mark.asyncio
async def test_fetch_schemas_merges_harness_and_memory_tools():
    harness = _client_with_tools([
        _mock_tool("bot.follow", "follow another player",
                   {"type": "object",
                    "properties": {"bot_guid": {"type": "integer"},
                                   "leader_guid": {"type": "integer"}},
                    "required": ["bot_guid", "leader_guid"]}),
    ])
    memory = _client_with_tools([
        _mock_tool("memory.recall", "recall recent memories",
                   {"type": "object",
                    "properties": {"bot_id": {"type": "string"},
                                   "query": {"type": "string"}},
                    "required": ["bot_id"]}),
    ])
    per_tool = await fetch_schemas(harness, memory)
    assert set(per_tool.keys()) == {"bot.follow", "memory.recall"}
    assert per_tool["bot.follow"].source_mcp == "harness"
    assert per_tool["memory.recall"].source_mcp == "memory"
    # Verify the unwrap was applied.
    assert per_tool["bot.follow"].schema["properties"]["bot_guid"]["type"] == "integer"


@pytest.mark.asyncio
async def test_fetch_schemas_raises_on_malformed_input_schema():
    bad = MagicMock()
    bad.name = "x.bad"
    bad.description = ""
    bad.inputSchema = {"type": "object"}  # missing properties.args
    harness = _client_with_tools([bad])
    memory = _client_with_tools([])
    with pytest.raises(ValueError, match="missing required 'args'"):
        await fetch_schemas(harness, memory)


@pytest.mark.asyncio
async def test_fetch_schemas_handles_empty_description():
    harness = _client_with_tools([
        _mock_tool("y.tool", None,
                   {"type": "object", "properties": {}, "required": []}),
    ])
    memory = _client_with_tools([])
    per_tool = await fetch_schemas(harness, memory)
    assert per_tool["y.tool"].description == ""


def test_unwrap_fastmcp_args_resolves_ref_to_defs():
    raw = {
        "type": "object",
        "properties": {"args": {"$ref": "#/$defs/BotFollowArgs"}},
        "required": ["args"],
        "$defs": {
            "BotFollowArgs": {
                "type": "object",
                "properties": {
                    "bot_guid": {"type": "integer"},
                    "leader_guid": {"type": "integer"},
                },
                "required": ["bot_guid", "leader_guid"],
            }
        },
    }
    inner = unwrap_fastmcp_args(raw)
    # Must return the resolved schema, NOT the $ref pointer.
    assert "$ref" not in inner
    assert inner["properties"]["bot_guid"]["type"] == "integer"
    assert inner["required"] == ["bot_guid", "leader_guid"]


def test_unwrap_fastmcp_args_raises_on_ref_without_defs():
    raw = {
        "type": "object",
        "properties": {"args": {"$ref": "#/$defs/MissingDef"}},
        "required": ["args"],
        # No $defs block.
    }
    with pytest.raises(ValueError, match="cannot resolve.*MissingDef"):
        unwrap_fastmcp_args(raw)


def test_unwrap_fastmcp_args_raises_on_ref_def_not_present():
    raw = {
        "type": "object",
        "properties": {"args": {"$ref": "#/$defs/UnknownDef"}},
        "required": ["args"],
        "$defs": {"OtherDef": {"type": "object"}},
    }
    with pytest.raises(ValueError, match="cannot resolve.*UnknownDef"):
        unwrap_fastmcp_args(raw)


def test_unwrap_fastmcp_args_resolves_nested_refs_inside_args():
    """Nested $refs (e.g., enum fields) must be inlined too — V3.1 deploy bug."""
    raw = {
        "type": "object",
        "properties": {"args": {"$ref": "#/$defs/SetStrategyArgs"}},
        "required": ["args"],
        "$defs": {
            "SetStrategyArgs": {
                "type": "object",
                "properties": {
                    "bot_guid": {"type": "integer"},
                    "strategy": {"type": "string"},
                    "bot_state": {"$ref": "#/$defs/_BotState"},
                },
                "required": ["bot_guid", "strategy"],
            },
            "_BotState": {
                "type": "string",
                "enum": ["combat", "non_combat", "dead", "all"],
            },
        },
    }
    inner = unwrap_fastmcp_args(raw)
    # The bot_state field must be fully resolved — no $ref pointer remains.
    bot_state = inner["properties"]["bot_state"]
    assert "$ref" not in bot_state, f"nested $ref not resolved: {bot_state}"
    assert bot_state["type"] == "string"
    assert set(bot_state["enum"]) == {"combat", "non_combat", "dead", "all"}


def test_unwrap_fastmcp_args_resolves_ref_inside_list():
    """$refs inside array items must also be resolved (e.g., relations field)."""
    raw = {
        "type": "object",
        "properties": {"args": {"$ref": "#/$defs/WriteArgs"}},
        "required": ["args"],
        "$defs": {
            "WriteArgs": {
                "type": "object",
                "properties": {
                    "relations": {
                        "type": "array",
                        "items": {"$ref": "#/$defs/Relation"},
                    },
                },
            },
            "Relation": {
                "type": "object",
                "properties": {"from": {"type": "string"}, "to": {"type": "string"}},
            },
        },
    }
    inner = unwrap_fastmcp_args(raw)
    item_schema = inner["properties"]["relations"]["items"]
    assert "$ref" not in item_schema
    assert item_schema["type"] == "object"
    assert "from" in item_schema["properties"]


def test_unwrap_fastmcp_args_detects_ref_cycles():
    """A self-referential $ref must raise, not infinite loop."""
    raw = {
        "type": "object",
        "properties": {"args": {"$ref": "#/$defs/Cycle"}},
        "required": ["args"],
        "$defs": {
            "Cycle": {"$ref": "#/$defs/Cycle"},
        },
    }
    with pytest.raises(ValueError, match="cycle detected"):
        unwrap_fastmcp_args(raw)


def test_unwrap_fastmcp_args_no_defs_block_with_refs():
    """Tool with $ref but no $defs block — fail loud."""
    raw = {
        "type": "object",
        "properties": {"args": {"$ref": "#/$defs/Missing"}},
        "required": ["args"],
    }
    with pytest.raises(ValueError, match="Missing not present in"):
        unwrap_fastmcp_args(raw)


def test_v372_decision_metadata_includes_wakeup_in_ms():
    """V3.7.2: wakeup_in_ms must appear in every branch of compose_oneof so the
    llama-server grammar permits the LLM to emit it. Without this, the field is
    grammar-rejected regardless of prompt wording (V3.7.1 soak proved this with
    70/70 default rate)."""
    from brain_sidecar.schema_builder import compose_oneof
    union = compose_oneof({"bot.follow": _follow_entry()})
    for branch in union["oneOf"]:
        props = branch["properties"]
        assert "wakeup_in_ms" in props, (
            f"Branch {branch.get('title')} must include wakeup_in_ms in properties"
        )
        # Field accepts integer-in-range OR null
        schema = props["wakeup_in_ms"]
        assert "anyOf" in schema
        types_seen = set()
        for sub in schema["anyOf"]:
            if "type" in sub:
                types_seen.add(sub["type"])
            if sub.get("type") == "integer":
                assert sub.get("minimum") == 60000
                assert sub.get("maximum") == 600000
        assert types_seen >= {"integer", "null"}
        # NOT in required (LLM may omit)
        assert "wakeup_in_ms" not in branch.get("required", [])
