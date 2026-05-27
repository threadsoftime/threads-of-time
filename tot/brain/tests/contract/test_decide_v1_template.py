"""V3.6: decide_v1.txt parses with EXAMPLE C and the rest of the template."""
from __future__ import annotations

import json
from pathlib import Path


def _load_template() -> str:
    """Locate prompts/decide_v1.txt relative to the package."""
    pkg_root = Path(__file__).resolve().parents[2]
    return (pkg_root / "prompts" / "decide_v1.txt").read_text(encoding="utf-8")


def test_decide_v1_template_str_format_succeeds():
    """The template str.format()-s cleanly against representative context."""
    tmpl = _load_template()
    rendered = tmpl.format(
        state_json=json.dumps({"self": {"level": 25}}),
        goals_json="[]",
        memories_json="[]",
        recent_decisions_json="[]",
        hot_inputs_json="{}",
        tools_summary="[]",
    )
    # EXAMPLE C present + uses bot.send_chat
    assert "EXAMPLE C" in rendered
    assert "bot.send_chat" in rendered


def test_decide_v1_example_c_brace_doubled_for_at_cap_whisper():
    """EXAMPLE C literal braces are doubled per kb_87a7eade rule #16."""
    tmpl = _load_template()
    # Find the EXAMPLE C section
    idx = tmpl.find("EXAMPLE C")
    assert idx >= 0
    section = tmpl[idx:]
    # The example contains JSON {} which must be {{}} so str.format doesn't choke
    assert "{{" in section
    assert "}}" in section


def test_v373_decision_schema_literal_drops_null_suffix():
    """V3.7.3: schema literal drops the |null suffix so the LLM doesn't see null
    as an option in the prompt-visible description. The grammar schema
    (schema_builder._DECISION_METADATA_PROPS) still permits null via anyOf for
    runtime flexibility."""
    from brain_sidecar.decide import _DECISION_SCHEMA_LITERAL
    assert "wakeup_in_ms" in _DECISION_SCHEMA_LITERAL
    assert "60000" in _DECISION_SCHEMA_LITERAL
    assert "600000" in _DECISION_SCHEMA_LITERAL
    # V3.7.3: |null suffix removed
    assert "<60000..600000>|null" not in _DECISION_SCHEMA_LITERAL
    assert "<60000..600000>" in _DECISION_SCHEMA_LITERAL  # bare range, no |null
    # Old name fully removed
    assert "wakeup_at_ms" not in _DECISION_SCHEMA_LITERAL
