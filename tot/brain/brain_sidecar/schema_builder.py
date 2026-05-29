"""V3.1: build the LLM grammar + prompt summary from live MCP tools/list.

Pull tools/list from harness:8099 + memory:8090 at brain startup; unwrap the
FastMCP `{args: ...}` envelope; compose a discriminated-union JSON schema
covering every tool plus a no_op branch; render a human-readable B-style
prompt summary.
"""
from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Any

log = logging.getLogger(__name__)


def _resolve_refs(node: Any, defs: dict[str, Any], stack: tuple[str, ...] = ()) -> Any:
    """Walk a JSON schema tree, replacing every {"$ref": "#/$defs/X"} with defs[X].

    Cycle-safe via `stack` of currently-resolving def names. Returns a new
    structure; does not mutate `node` in place.
    """
    if isinstance(node, dict):
        ref = node.get("$ref")
        if ref is not None:
            prefix = "#/$defs/"
            if not ref.startswith(prefix):
                raise ValueError(
                    f"cannot resolve unsupported $ref form: {ref!r} (expected #/$defs/...)"
                )
            def_name = ref[len(prefix):]
            if def_name in stack:
                raise ValueError(
                    f"cannot resolve $ref {ref!r}: cycle detected (stack={stack})"
                )
            if def_name not in defs:
                raise ValueError(
                    f"cannot resolve $ref {ref!r}: {def_name} not present in $defs "
                    f"(available: {sorted(defs.keys())})"
                )
            # Recurse into the resolved def in case it has nested $refs.
            return _resolve_refs(defs[def_name], defs, stack + (def_name,))
        # Walk dict values
        return {k: _resolve_refs(v, defs, stack) for k, v in node.items()}
    if isinstance(node, list):
        return [_resolve_refs(item, defs, stack) for item in node]
    return node


def unwrap_fastmcp_args(input_schema: dict[str, Any]) -> dict[str, Any]:
    """Strip the FastMCP `{args: ...}` envelope and recursively resolve all $refs.

    Both tot-harness and memory-sidecar register their tools via FastMCP
    with handler signatures `_handler(ctx, args: SchemaModel)`. FastMCP exposes
    this as an inputSchema with `args` as the sole top-level property; the
    real per-tool args are nested under `properties.args` and may contain
    `$ref` pointers into `inputSchema["$defs"]` for nested types (enums,
    sub-models). This function:

      1. Pulls `properties.args` (raises ValueError if missing).
      2. Recursively walks the result, replacing every `{"$ref": "#/$defs/X"}`
         with the actual `inputSchema["$defs"]["X"]` content.

    The returned schema is fully self-contained — no `$ref` pointers remain,
    so it can be embedded directly into `compose_oneof`'s discriminated union
    without llama-server's json_schema-to-grammar compiler choking on
    unresolvable references.

    Raises ValueError if any required structure is missing or if a $ref cycle
    is detected.
    """
    props = input_schema.get("properties", {})
    if "args" not in props:
        raise ValueError(
            f"inputSchema missing required 'args' property: keys={list(props.keys())}"
        )
    defs = input_schema.get("$defs", {})
    return _resolve_refs(props["args"], defs)


@dataclass(frozen=True)
class ToolEntry:
    """One row in the per_tool dict: description + unwrapped schema + which MCP it came from."""
    description: str
    schema: dict[str, Any]
    source_mcp: str  # "harness" or "memory"


_DECISION_METADATA_PROPS = {
    "confidence": {"type": "number", "minimum": 0, "maximum": 1},
    "reasoning": {"type": "string"},
    # V3.7.2: optional wakeup_in_ms (delta in ms). Without this field in
    # _DECISION_METADATA_PROPS the llama-server grammar (additionalProperties: False
    # + strict: True) would reject emission of the field. V3.7.1 soak proved this:
    # 70/70 organic_wakeup LLM ticks defaulted because the LLM could not emit
    # the value. Field is optional (not in `required`); None still routes
    # through the loop.py clamp default.
    "wakeup_in_ms": {
        "anyOf": [
            {"type": "integer", "minimum": 60000, "maximum": 600000},
            {"type": "null"},
        ],
    },
}


def compose_oneof(per_tool: dict[str, ToolEntry]) -> dict[str, Any]:
    """Build the discriminated-union JSON schema covering Decision shape.

    Branches:
    - One `no_op` branch with tool=null, args=null.
    - One `action:<tool>` branch per entry in `per_tool`, with tool fixed
      to the tool name (via const) and args fixed to the unwrapped schema.

    The result is suitable for llama-server's `response_format.json_schema.schema`.
    """
    branches: list[dict[str, Any]] = [{
        "title": "no_op",
        "type": "object",
        "properties": {
            "kind": {"const": "no_op"},
            "tool": {"type": "null"},
            "args": {"type": "null"},
            **_DECISION_METADATA_PROPS,
        },
        "required": ["kind", "tool", "args", "confidence", "reasoning"],
        "additionalProperties": False,
    }]
    for tool_name, entry in sorted(per_tool.items()):
        branches.append({
            "title": f"action:{tool_name}",
            "type": "object",
            "properties": {
                "kind": {"const": "action"},
                "tool": {"const": tool_name},
                "args": entry.schema,
                **_DECISION_METADATA_PROPS,
            },
            "required": ["kind", "tool", "args", "confidence", "reasoning"],
            "additionalProperties": False,
        })
    return {"oneOf": branches}


_JSON_TO_PY_TYPE = {
    "integer": "int",
    "string": "str",
    "number": "float",
    "boolean": "bool",
    "object": "dict",
    "array": "list",
    "null": "None",
}


def _render_arg(name: str, schema: dict[str, Any], required: bool) -> str:
    """Render one arg as 'name: type' or 'name?: type' if optional."""
    py_type = _JSON_TO_PY_TYPE.get(schema.get("type", ""), "any")
    qmark = "" if required else "?"
    return f"{name}{qmark}: {py_type}"


async def fetch_schemas(harness_mcp: Any, memory_mcp: Any) -> dict[str, ToolEntry]:
    """Pull tools/list from both MCPs, unwrap, and merge into one dict.

    Raises ValueError if any tool's inputSchema does not follow the FastMCP
    `{args: ...}` envelope. Network errors propagate from list_tools().
    """
    harness_tools = await harness_mcp.list_tools()
    memory_tools = await memory_mcp.list_tools()

    per_tool: dict[str, ToolEntry] = {}
    for tool in harness_tools:
        per_tool[tool.name] = ToolEntry(
            description=tool.description or "",
            schema=unwrap_fastmcp_args(tool.inputSchema),
            source_mcp="harness",
        )
    for tool in memory_tools:
        per_tool[tool.name] = ToolEntry(
            description=tool.description or "",
            schema=unwrap_fastmcp_args(tool.inputSchema),
            source_mcp="memory",
        )
    return per_tool


def render_prompt_summary(per_tool: dict[str, ToolEntry]) -> str:
    """Render the B-style human-readable tool list for the LLM prompt.

    Example output line:
        bot.follow(bot_guid: int, leader_guid: int)
          — follow another player; durable

    Tools are listed sorted alphabetically by name. If a tool has an empty
    description, the trailing "— ..." is omitted.
    """
    lines: list[str] = []
    for tool_name, entry in sorted(per_tool.items()):
        props = entry.schema.get("properties", {})
        required = set(entry.schema.get("required", []))
        arg_strs = [_render_arg(n, s, n in required) for n, s in props.items()]
        sig = f"  {tool_name}({', '.join(arg_strs)})"
        lines.append(sig)
        if entry.description:
            lines.append(f"    — {entry.description}")
    return "\n".join(lines)
