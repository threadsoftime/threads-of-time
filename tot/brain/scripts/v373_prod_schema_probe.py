"""V3.7.3 production-equivalent probe.

Builds the same 48-branch decision_schema the live brain uses (via
schema_builder.compose_oneof against the live MCP tools/list), then hits
llama-server with the V3.7.3 prompt and counts wakeup_in_ms distribution.

If this probe shows 100% default rate (matching prod) → schema complexity is
the binding constraint, not the prompt.
If this probe shows variety → something else in the prod call path is the
binding constraint (e.g., real persona vs test persona; full hot_inputs JSON).
"""
import asyncio
import json
import os
import sys
import time
import urllib.request
from collections import Counter

# Ensure we can import the brain
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from brain_sidecar import schema_builder
from brain_sidecar.mcp_clients import McpClient

LLAMA_URL = "http://192.168.1.3:8080/v1/chat/completions"
HARNESS_URL = "http://192.168.1.3:8099/mcp/mcp"
MEMORY_URL = "http://192.168.1.3:8090/mcp/mcp"
HARNESS_BEARER = os.environ.get("HARNESS_BEARER", "")
MEMORY_BEARER = os.environ.get("MEMORY_BEARER", "")


SYSTEM = (
    "You are Casmina, a Human Paladin in World of Warcraft. A devoted tank.\n"
    "Your bot_guid is 1003. When a tool's args require bot_guid, "
    "use exactly this integer (1003). Do not use 0 or guess.\n"
    "Personality traits (0..1 scale unless noted): "
    "talkativeness=0.7, courage=0.9, greed=0.3, "
    "attitude_to_master=0.6 (range -1..1), "
    "party_invite_policy=accept_from_known.\n"
    "Your default dungeon role: tank or healer.\n"
    "\nYou are at max level (L25). XP from quests no longer matters. "
    "Your end-game preferences (0..1): "
    "pvp_appetite=0.4, raid_appetite=0.9, completionist_streak=0.5, "
    "gold_motivation=0.4, profession_appetite=0.2.\n"
    "Available end-game activities on this server: Black Fathom Deeps raid, "
    "battlegrounds, dungeon farming, profession crafting, gold/AH play, zone completion.\n"
    "\nYour goals shape what you do across ticks. If you have no active "
    "goals, create one now via goals.create — pick something concrete that "
    'fits your personality (e.g., "reach level 25", "earn 5 gold by '
    'tomorrow", "run BFD with a group"). Goals you write here are your own '
    "— the brain owns them, they persist across ticks, and you reference "
    "them in future decisions.\n"
    "\nYou woke up on your own — no one is asking you anything, no "
    "combat is happening, no invitations are pending. This is your "
    "own time. Decide what YOU want to do next based on your "
    "personality, your goals, and your current situation. If nothing "
    "important needs doing, that's a valid choice — emit no_op.\n"
    "Set wakeup_in_ms to a number of milliseconds reflecting "
    "your current engagement: shorter (60000-120000) if you're "
    "actively pursuing something or expecting an event soon; "
    "medium (180000-360000) if you're between activities; "
    "longer (480000-600000) if you're settled and content. Vary "
    "it based on your situation — don't pick the same value "
    "every time.\n"
    "Look at your recent decisions in this prompt. If you've done "
    "the same action 3+ times in a row, deliberately pick something "
    "different — a real character varies their activities. Repetition "
    "is fine; identical repetition isn't.\n"
    "\nYou make ONE decision per call. Respond with ONLY a single JSON object "
    "matching this exact schema, and nothing else (no prose, no markdown, no preamble):\n"
    '{"kind": "action"|"no_op", "tool": "<tool_name>"|null, '
    '"args": {<tool-specific>}|null, "confidence": 0.0..1.0, '
    '"reasoning": "<one sentence>", "wakeup_in_ms": <60000..600000>}'
)


async def build_schema():
    """Build the same schema the brain builds at startup."""
    harness = McpClient(HARNESS_URL, HARNESS_BEARER)
    memory = McpClient(MEMORY_URL, MEMORY_BEARER)
    per_tool = await schema_builder.fetch_schemas(harness, memory)
    return schema_builder.compose_oneof(per_tool), per_tool


async def main():
    schema, per_tool = await build_schema()
    n_branches = len(schema["oneOf"])
    print(f"built schema with {n_branches} branches (vs probe's 2 branches)")
    tool_names = sorted(per_tool.keys())
    tools_summary = "\n".join(f"  {n}" for n in tool_names)

    user = (
        "CURRENT STATE: {\"self\":{\"level\":25,\"in_group\":false}}\n"
        "ACTIVE GOALS: []\n"
        "RECENT MEMORIES: []\n"
        "YOUR LAST 3 DECISIONS: []\n"
        "WHAT JUST CHANGED: {}\n"
        f"AVAILABLE TOOLS (call only one per decision):\n{tools_summary}\n"
        "\nDecide ONE action."
    )

    results = []
    print(f"\n=== probing with full 48-branch schema, V3.7.3 prompt, 8 calls ===\n")
    for i in range(8):
        payload = {
            "model": "qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf",
            "messages": [
                {"role": "system", "content": SYSTEM},
                {"role": "user", "content": user},
            ],
            "max_tokens": 400,
            "temperature": 0.7,
            "response_format": {
                "type": "json_schema",
                "json_schema": {"name": "decision", "schema": schema, "strict": True},
            },
        }
        req = urllib.request.Request(
            LLAMA_URL,
            data=json.dumps(payload).encode(),
            headers={"Content-Type": "application/json"},
        )
        t0 = time.time()
        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                data = json.loads(resp.read())
            latency = (time.time() - t0) * 1000
            raw = data["choices"][0]["message"]["content"]
            try:
                parsed = json.loads(raw)
                val = parsed.get("wakeup_in_ms")
                tool = parsed.get("tool") or "(no_op)"
                print(f"call {i+1} ({latency:.0f}ms): wakeup_in_ms={val}  tool={tool}")
                results.append(val)
            except json.JSONDecodeError:
                print(f"call {i+1}: parse error; raw[:200]={raw[:200]}")
                results.append("PARSE_ERROR")
        except Exception as e:
            print(f"call {i+1}: error {e}")
            results.append("ERROR")
    print()
    print("=== summary (V3.7.3 prompt, FULL production schema) ===")
    dist = Counter(results)
    print(f"distribution: {dict(dist)}")
    print(f"distinct values: {len([k for k in dist if isinstance(k, int)])}")
    default_count = dist.get(180000, 0)
    valid_count = sum(c for k, c in dist.items() if isinstance(k, int))
    if valid_count > 0:
        print(f"default-rate (180000): {default_count}/{valid_count} = {100*default_count/valid_count:.1f}%")


if __name__ == "__main__":
    asyncio.run(main())
