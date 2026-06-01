"""Parity test: pydantic schemas in tool_schemas.py vs the C++ adapter sources.

This test exists because V0.2-mcp shipped with 4-7 silent drift bugs (the
pydantic schema named the field one thing while the adapter parsed
another). The HTTP `/v1/*` surface has no pydantic gate so those bugs
slipped past every smoke test. See kb_642162c3 + kb_6950a902 item 7.

What this test guarantees:

  1. Every adapter `args.contains("X")` field is declared as a REQUIRED
     pydantic field on the matching schema. (catches B1/B2/B3/B5.)

  2. Every pydantic field is referenced by the adapter somewhere
     (`args.contains`, `args.value`, or `args["X"]`). Otherwise it
     would be silently ignored, which is what trapped B7. The LLM
     would think the field works and we'd never know.

  3. Optional adapter fields (`args.value("X", default)`) MAY appear in
     the schema; not required, but if they do they must use the same
     name.

When this test fails, the fix lives in `tool_schemas.py` — change the
pydantic model to match the C++ adapter, not the other way around.
The C++ adapter is the source of truth (kb_6950a902 closing paragraph).
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Iterable

import pytest

from harness_daemon.tool_schemas import TOOL_SCHEMAS


# tests/test_mcp_schemas.py -> parents[3] = repo root
ADAPTERS_DIR = (
    Path(__file__).resolve().parents[3]
    / "modules" / "mod-harness-bridge" / "src" / "Adapters"
)

# Tools whose schemas have no matching C++ adapter (daemon-direct only).
DAEMON_DIRECT_TOOLS = {"obs.query_db"}

# Fields the daemon injects into args before forwarding to the adapter,
# so they should NOT appear in the LLM-facing pydantic schema. See
# harness_daemon.ac_client.dispatch().
DAEMON_INJECTED_FIELDS = {
    "gm.run_console": {"request_id"},
}

# Fields that the adapter checks via args.contains() in an OR-gate
# (i.e. "at least one of X or Y is required") rather than a hard
# per-field gate.  The _extract_required() regex cannot distinguish the
# OR-gate from a hard-required check, so these fields appear as
# "adapter_required" when they are actually OR-optional.  We subtract
# them from the required-field assertion so the pydantic schema can
# correctly declare them Optional (the adapter enforces the OR at
# runtime).
# Fields in memory.* adapters that `args.contains("X")` checks as an
# optional-presence guard (body-building block: "if present, include in
# the HTTP body"), NOT as a hard-required gate.  The _extract_required()
# regex cannot distinguish these two patterns, so we exclude them from
# the required-field assertion here.  Pydantic correctly marks them
# Optional; the adapter silently skips absent fields rather than rejecting.
SCHEMA_OR_REQUIRED_FIELDS: dict[str, set[str]] = {
    # bot.queue_for_dungeon: hard-required = bot_guid, dungeon_id.
    # roles_mask uses the ternary presence-guard pattern:
    #   args.contains("roles_mask") ? args["roles_mask"].get<int>() : 0
    # The _extract_required() regex matches args.contains() regardless of
    # whether it is a hard-required gate or a defaulted presence check, so
    # roles_mask appears in adapter_required even though the adapter falls
    # back to 0 (auto-detect from talent spec) when it is absent.
    "bot.queue_for_dungeon": {"roles_mask"},
    # memory.write: hard-required = bot_guid, content_text, episode_type, timestamp.
    # Everything else is body-building (optional).
    "memory.write": {"salience_hint", "entities", "metadata", "source"},
    # memory.recall: hard-required = bot_guid, query_text.
    # Scoring weights + filters are body-building (optional).
    "memory.recall": {
        "top_k", "entity_names", "episode_types", "time_filter",
        "alpha", "beta", "gamma", "delta", "mmr_lambda",
    },
    # memory.search: hard-required = bot_guid + (query_text OR query_vec).
    # Both query fields appear in an OR-gate; top_k is body-building.
    "memory.search": {"query_text", "query_vec", "top_k"},
    # memory.list: hard-required = bot_guid.
    # All filter + pagination params are query-string building (optional).
    "memory.list": {
        "episode_type", "entity_name", "after", "before", "limit", "offset",
    },
    # memory.update: hard-required = bot_guid, episode_id.
    # Mutable fields are OR-required (adapter enforces "at least one" at runtime).
    "memory.update": {"content_text", "salience_score", "metadata"},
}


def _tool_to_adapter_filename(tool_name: str) -> str:
    """e.g. `gm.read_console_output` -> `GmReadConsoleOutputAdapter.cpp`."""
    ns, rest = tool_name.split(".", 1)
    parts = [ns] + rest.split("_")
    return "".join(p.title() for p in parts) + "Adapter.cpp"


# Captures: args.contains("X"), args.value("X", ...), args["X"]
_ARG_FIELD_RE = re.compile(r'args(?:\.contains|\.value|\[)\s*\(?\s*"([^"]+)"')
# Captures: `for (auto const& key : {"X", "Y", "Z"})` — the GmTeleport pattern
_FOR_BRACED_RE = re.compile(
    r'for\s*\([^)]*:\s*\{\s*((?:"[^"]+"\s*,?\s*)+)\}\s*\)', re.DOTALL,
)
_STRING_LITERAL_RE = re.compile(r'"([^"]+)"')


def _extract_required(cpp: str) -> set[str]:
    """Fields the adapter rejects on absence (args.contains + for-iter)."""
    required: set[str] = set()
    for m in re.finditer(r'args\.contains\(\s*"([^"]+)"\s*\)', cpp):
        required.add(m.group(1))
    for m in _FOR_BRACED_RE.finditer(cpp):
        required.update(_STRING_LITERAL_RE.findall(m.group(1)))
    return required


def _extract_optional(cpp: str) -> set[str]:
    """Fields the adapter reads via args.value("X", default)."""
    return {m.group(1) for m in re.finditer(
        r'args\.value\(\s*"([^"]+)"', cpp,
    )}


def _extract_all_touched(cpp: str) -> set[str]:
    """All field names the adapter accesses in any form."""
    return set(_ARG_FIELD_RE.findall(cpp)) | _extract_required(cpp)


def _adapter_path(tool_name: str) -> Path:
    return ADAPTERS_DIR / _tool_to_adapter_filename(tool_name)


def _pydantic_required(schema_cls) -> set[str]:
    return {n for n, f in schema_cls.model_fields.items() if f.is_required()}


def _pydantic_all(schema_cls) -> set[str]:
    return set(schema_cls.model_fields.keys())


def _adapter_tools() -> Iterable[str]:
    """Every tool in TOOL_SCHEMAS that has a corresponding C++ adapter."""
    return sorted(name for name in TOOL_SCHEMAS if name not in DAEMON_DIRECT_TOOLS)


@pytest.fixture(scope="module")
def adapters_available() -> bool:
    """Skip the file-grep tests when adapter sources aren't checked out
    (e.g. CI sandbox without the modules/ submodule)."""
    return ADAPTERS_DIR.exists() and any(ADAPTERS_DIR.glob("*Adapter.cpp"))


@pytest.mark.parametrize("tool_name", list(_adapter_tools()))
def test_schema_has_all_adapter_required_fields(tool_name, adapters_available):
    """Pydantic REQUIRED fields must include every adapter args.contains() field."""
    if not adapters_available:
        pytest.skip("adapter sources not checked out at modules/mod-harness-bridge/")
    path = _adapter_path(tool_name)
    if not path.exists():
        pytest.fail(f"no adapter file for tool {tool_name!r} at {path}")

    cpp = path.read_text()
    adapter_required = (
        _extract_required(cpp)
        - DAEMON_INJECTED_FIELDS.get(tool_name, set())
        - SCHEMA_OR_REQUIRED_FIELDS.get(tool_name, set())
    )
    schema_cls, _ = TOOL_SCHEMAS[tool_name]
    schema_required = _pydantic_required(schema_cls)

    missing = adapter_required - schema_required
    assert not missing, (
        f"{tool_name}: pydantic {schema_cls.__name__} is missing REQUIRED "
        f"adapter fields {sorted(missing)!r}. "
        f"Adapter {path.name} requires {sorted(adapter_required)!r}; "
        f"schema declares required {sorted(schema_required)!r}. "
        f"Fix: add the missing field(s) to tool_schemas.py "
        f"with `= Field(..., description='...')`."
    )


@pytest.mark.parametrize("tool_name", list(_adapter_tools()))
def test_schema_has_no_phantom_fields(tool_name, adapters_available):
    """Every pydantic field must be referenced by the adapter somewhere.

    A schema field the adapter never reads is silently ignored — the LLM
    sees a documented arg, sends it, and nothing happens. This trapped
    B7 (since_ts vs since_ts_ms in obs.get_combat_log).
    """
    if not adapters_available:
        pytest.skip("adapter sources not checked out at modules/mod-harness-bridge/")
    path = _adapter_path(tool_name)
    if not path.exists():
        pytest.fail(f"no adapter file for tool {tool_name!r} at {path}")

    cpp = path.read_text()
    touched = _extract_all_touched(cpp)
    schema_cls, _ = TOOL_SCHEMAS[tool_name]
    schema_fields = _pydantic_all(schema_cls)

    phantom = schema_fields - touched - DAEMON_INJECTED_FIELDS.get(tool_name, set())
    assert not phantom, (
        f"{tool_name}: pydantic {schema_cls.__name__} declares fields "
        f"{sorted(phantom)!r} that the adapter NEVER reads. These would "
        f"be silently ignored. Adapter {path.name} only touches "
        f"{sorted(touched)!r}. Fix: either rename to the adapter's "
        f"field name, or remove the field from the schema."
    )


@pytest.mark.parametrize("tool_name", list(_adapter_tools()))
def test_schema_required_vs_optional_matches_adapter(tool_name, adapters_available):
    """A pydantic-required field that the adapter only reads optionally
    (args.value with a default) is a bug — the LLM is forced to supply
    a value the adapter would otherwise default. The reverse (schema-
    optional, adapter-required) is caught by the first test."""
    if not adapters_available:
        pytest.skip("adapter sources not checked out at modules/mod-harness-bridge/")
    path = _adapter_path(tool_name)
    if not path.exists():
        pytest.fail(f"no adapter file for tool {tool_name!r} at {path}")

    cpp = path.read_text()
    adapter_required = (
        _extract_required(cpp)
        - DAEMON_INJECTED_FIELDS.get(tool_name, set())
        - SCHEMA_OR_REQUIRED_FIELDS.get(tool_name, set())
    )
    adapter_optional = _extract_optional(cpp)
    schema_cls, _ = TOOL_SCHEMAS[tool_name]
    schema_required = _pydantic_required(schema_cls)

    # Fields the schema marks required, but the adapter treats as optional.
    # adapter_required already excludes daemon-injected and OR-required
    # fields, so this catches the real over-strict case.
    wrongly_required = schema_required & adapter_optional - adapter_required
    assert not wrongly_required, (
        f"{tool_name}: pydantic {schema_cls.__name__} marks "
        f"{sorted(wrongly_required)!r} as REQUIRED, but adapter "
        f"{path.name} reads them with args.value(...,default). Schema "
        f"should make these Optional[...] = Field(<default>, ...)."
    )


def test_adapter_filename_mapping_is_round_trip():
    """Sanity: every adapter we expect on disk has a matching tool name."""
    if not ADAPTERS_DIR.exists():
        pytest.skip("adapter sources not checked out")
    expected_filenames = {_tool_to_adapter_filename(t) for t in _adapter_tools()}
    on_disk = {p.name for p in ADAPTERS_DIR.glob("*Adapter.cpp")}
    # We allow on_disk to be a superset (new adapter shipped without schema yet
    # — caught by test_tool_schemas_covers_full_registry).
    missing_on_disk = expected_filenames - on_disk
    assert not missing_on_disk, (
        f"TOOL_SCHEMAS names map to adapter files not on disk: "
        f"{sorted(missing_on_disk)!r}. Check the camel-case translation in "
        f"_tool_to_adapter_filename()."
    )
