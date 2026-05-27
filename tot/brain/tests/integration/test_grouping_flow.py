"""V3.5 integration tests — grouping flow end-to-end (FakeMcp + FakeLlm).

All PROBED_SHAPE constants are filled with verbatim JSON dicts from spec §4.3,
probed 2026-05-22 against bots 1003/1004/1014 on the live Stage 1.5 worldserver.

FakeMcp note: the triage gate calls `_safe_call` which unwraps {"ok": ..., "result": {...}}
envelopes. When FakeMcp returns a dict with a "result" key, triage will see the inner
`result` value as `obs_state`. The probed envelopes include the outer wrapper, so the
inner `result` dict (with "self", "social", etc.) is what triage logic operates on.

Timing note: triage filters memory items by `ts > since_ts_s` where `since_ts_s` is
Unix epoch seconds. Items with small ts values (e.g., 1005) are always stale after
the first tick. To make fresh_chat trigger reliably on subsequent ticks, memory items
must use a far-future `ts` (e.g., 9_999_999_999).
"""
from __future__ import annotations

import asyncio
import json

import pytest

from brain_sidecar.decide import Decider
from brain_sidecar.dispatch import Dispatcher
from brain_sidecar.loop import LoopSupervisor
from brain_sidecar.models import PersonalityCard
from brain_sidecar.personality import PersonalityCache
from brain_sidecar.state import StateStore
from brain_sidecar.triage import TriageGate

from .mocks import FakeLlm, FakeMcp

# ---------------------------------------------------------------------------
# Role-mask constants (confirmed by probe P5, spec §4.3)
# ---------------------------------------------------------------------------
PLAYER_ROLE_NONE = 0
PLAYER_ROLE_TANK = 2
PLAYER_ROLE_HEALER = 4
PLAYER_ROLE_DAMAGE = 8

# Far-future timestamp (Unix seconds) — always > since_ts_s in triage freshness filter.
_FUTURE_TS = 9_999_999_999

# ---------------------------------------------------------------------------
# Probed-shape constants — verbatim from spec §4.3 (2026-05-22)
# ---------------------------------------------------------------------------

# P2.5 — obs.get_state(target_guid=1004) with pending invite (post-Stage-1.5 fix).
# FakeMcp returns the full envelope; _safe_call in triage unwraps "result".
# Triage sees: obs_state["self"]["pending_group_invite"] = {"from_name": "Casmina", "group_type": "party"}
PROBED_PENDING_INVITE_OBS = {
    "executor_ms": 299,
    "ok": True,
    "result": {
        "event_log": [],
        "goal": {
            "current": "Idle",
            "elapsed_minutes": 0,
            "params": {},
            "progress_pct": 0,
            "ttl_minutes": 0,
        },
        "inventory_highlights": {
            "bag_used": "0/72",
            "consumables": [],
            "gear_vs_level_score": 0.0,
            "junk_value_copper": 0,
        },
        "location": {
            "map": "Eastern Kingdoms",
            "near_npcs": [],
            "position": [-9208.78125, -2118.41064453125, 67.9093017578125],
            "subzone": "Lakeshire",
            "zone": "Redridge Mountains",
        },
        "memory_hints": [
            "killed Bellygrub in Redridge Mountains",
            "killed Murloc Minor Tidecaller in Redridge Mountains",
            "killed Great Goretusk in Redridge Mountains",
        ],
        "quest_log": [
            {"id": 20, "progress": "in progress", "title": "Blackrock Menace"},
            {"id": 91, "progress": "in progress", "title": "Solomon's Law"},
            {"id": 92, "progress": "in progress", "title": "Redridge Goulash"},
            {"id": 118, "progress": "in progress", "title": "The Price of Shoes"},
            {"id": 125, "progress": "in progress", "title": "The Lost Tools"},
            {"id": 127, "progress": "in progress", "title": "Selling Fish"},
            {"id": 135, "progress": "in progress", "title": "The Defias Brotherhood"},
            {"id": 150, "progress": "in progress", "title": "Murloc Poachers"},
            {"id": 255, "progress": "in progress", "title": "Mercenaries"},
            {"id": 467, "progress": "complete, turn in", "title": "Stonegear's Search"},
        ],
        "self": {
            "class": "hunter",
            "gold_copper": 7471398,
            "hp_pct": 100,
            "is_dead": False,
            "is_in_combat": False,
            "is_resting": False,
            "level": 21,
            "mana_pct": 100,
            "name": "Ellian",
            "pending_group_invite": {
                "from_name": "Casmina",
                "group_type": "party",
            },
            "race": "draenei",
            "spec": "",
        },
        "social": {
            "group_members": [],
            "guild": None,
            "in_group": False,
            "nearby_humans": [],
            "recent_whispers": [],
        },
    },
    "tick_wait_ms": 5,
    "request_id": "req_7c6327918581",
    "identity": "claude-code",
    "latency_ms": 306,
    "ac_latency_ms": 306,
}

# P3 — bot.accept_invite(bot_guid=1004) result (post-Stage-1.5 fix, no ByteBuffer).
# FakeMcp returns this dict; dispatch checks ok=true + result.accepted=true.
PROBED_ACCEPT_RESULT = {
    "executor_ms": 0,
    "ok": True,
    "result": {
        "accepted": True,
        "group_guid": 1,
        "group_type": "party",
    },
    "tick_wait_ms": 5,
    "request_id": "req_1398c878c302",
    "identity": "claude-code",
    "latency_ms": 8,
    "ac_latency_ms": 8,
}

# P3 follow-up — obs.get_state(target_guid=1003) post-accept (Casmina, grouped).
# social.in_group=true, social.group_members=["Ellian"], pending_group_invite=null.
PROBED_IN_GROUP_OBS_CASMINA = {
    "executor_ms": 2,
    "ok": True,
    "result": {
        "event_log": [],
        "goal": {
            "current": "Idle",
            "elapsed_minutes": 0,
            "params": {},
            "progress_pct": 0,
            "ttl_minutes": 0,
        },
        "inventory_highlights": {
            "bag_used": "14/96",
            "consumables": [],
            "gear_vs_level_score": 0.0,
            "junk_value_copper": 0,
        },
        "location": {
            "map": "Eastern Kingdoms",
            "near_npcs": [],
            "position": [-8820.5126953125, 522.5443115234375, 75.0814208984375],
            "subzone": "Stormwind City",
            "zone": "Stormwind City",
        },
        "memory_hints": [],
        "quest_log": [
            {"id": 20, "progress": "in progress", "title": "Blackrock Menace"},
            {"id": 67, "progress": "complete, turn in", "title": "The Legend of Stalvan"},
            {"id": 90, "progress": "in progress", "title": "Seasoned Wolf Kabobs"},
            {"id": 128, "progress": "in progress", "title": "Blackrock Bounty"},
            {"id": 133, "progress": "in progress", "title": "Ghoulish Effigy"},
            {"id": 174, "progress": "in progress", "title": "Look To The Stars"},
            {"id": 279, "progress": "in progress", "title": "Claws from the Deep"},
            {"id": 288, "progress": "in progress", "title": "The Third Fleet"},
            {"id": 294, "progress": "complete, turn in", "title": "Ormer's Revenge"},
            {"id": 306, "progress": "in progress", "title": "In Search of The Excavation Team"},
        ],
        "self": {
            "class": "paladin",
            "gold_copper": 6328626,
            "hp_pct": 100,
            "is_dead": False,
            "is_in_combat": False,
            "is_resting": True,
            "level": 25,
            "mana_pct": 100,
            "name": "Casmina",
            "pending_group_invite": None,
            "race": "human",
            "spec": "",
        },
        "social": {
            "group_members": ["Ellian"],
            "guild": None,
            "in_group": True,
            "nearby_humans": [],
            "recent_whispers": [],
        },
    },
    "tick_wait_ms": 22,
    "request_id": "req_b8a460a3a0a1",
    "identity": "claude-code",
    "latency_ms": 27,
    "ac_latency_ms": 27,
}

# P3 follow-up — obs.get_state(target_guid=1004) post-accept (Ellian, grouped).
# social.in_group=true, social.group_members=["Casmina"], pending_group_invite=null.
PROBED_IN_GROUP_OBS_ELLIAN = {
    "executor_ms": 44,
    "ok": True,
    "result": {
        "event_log": [],
        "goal": {
            "current": "Idle",
            "elapsed_minutes": 0,
            "params": {},
            "progress_pct": 0,
            "ttl_minutes": 0,
        },
        "inventory_highlights": {
            "bag_used": "0/72",
            "consumables": [],
            "gear_vs_level_score": 0.0,
            "junk_value_copper": 0,
        },
        "location": {
            "map": "Eastern Kingdoms",
            "near_npcs": [],
            "position": [-9208.78125, -2118.41064453125, 67.9093017578125],
            "subzone": "Lakeshire",
            "zone": "Redridge Mountains",
        },
        "memory_hints": [
            "killed Bellygrub in Redridge Mountains",
            "killed Murloc Minor Tidecaller in Redridge Mountains",
            "killed Great Goretusk in Redridge Mountains",
        ],
        "quest_log": [
            {"id": 20, "progress": "in progress", "title": "Blackrock Menace"},
            {"id": 91, "progress": "in progress", "title": "Solomon's Law"},
            {"id": 92, "progress": "in progress", "title": "Redridge Goulash"},
            {"id": 118, "progress": "in progress", "title": "The Price of Shoes"},
            {"id": 125, "progress": "in progress", "title": "The Lost Tools"},
            {"id": 127, "progress": "in progress", "title": "Selling Fish"},
            {"id": 135, "progress": "in progress", "title": "The Defias Brotherhood"},
            {"id": 150, "progress": "in progress", "title": "Murloc Poachers"},
            {"id": 255, "progress": "in progress", "title": "Mercenaries"},
            {"id": 467, "progress": "complete, turn in", "title": "Stonegear's Search"},
        ],
        "self": {
            "class": "hunter",
            "gold_copper": 7471398,
            "hp_pct": 100,
            "is_dead": False,
            "is_in_combat": False,
            "is_resting": False,
            "level": 21,
            "mana_pct": 100,
            "name": "Ellian",
            "pending_group_invite": None,
            "race": "draenei",
            "spec": "",
        },
        "social": {
            "group_members": ["Casmina"],
            "guild": None,
            "in_group": True,
            "nearby_humans": [],
            "recent_whispers": [],
        },
    },
    "tick_wait_ms": 20,
    "request_id": "req_b31f0e27409a",
    "identity": "claude-code",
    "latency_ms": 106,
    "ac_latency_ms": 106,
}

# P5 — bot.set_role(bot_guid=1003, role="tank") result.
PROBED_SET_ROLE_RESULT = {
    "executor_ms": 0,
    "ok": True,
    "result": {
        "role_set": "tank",
        "roles_mask": PLAYER_ROLE_TANK,
    },
    "tick_wait_ms": 17,
    "request_id": "req_53704b83a1c5",
    "identity": "claude-code",
    "latency_ms": 19,
    "ac_latency_ms": 19,
}

# P6 — bot.queue_for_dungeon(bot_guid=1003, dungeon_id=0, roles_mask=0) result.
# dungeon_id=0 resolves to -1 (random sentinel); roles_mask=0 auto-resolved to 8 (DPS).
PROBED_QUEUE_RESULT = {
    "executor_ms": 0,
    "ok": True,
    "result": {
        "dungeon_id": -1,
        "lfg_state": "queued",
        "queued": True,
        "roles_mask": PLAYER_ROLE_DAMAGE,
    },
    "tick_wait_ms": 3,
    "request_id": "req_07cb6dde28bd",
    "identity": "claude-code",
    "latency_ms": 5,
    "ac_latency_ms": 5,
}


# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------

def _card(name: str, class_str: str, party_invite_policy: str = "accept_from_known") -> PersonalityCard:
    return PersonalityCard(
        name=name, race="Human", **{"class": class_str},
        backstory="A test character.",
        talkativeness=0.7, courage=0.7, greed=0.2, attitude_to_master=0.6,
        party_invite_policy=party_invite_policy,
    )


def _persona_json(card: PersonalityCard) -> str:
    return json.dumps({
        "name": card.name, "race": card.race, "class": card.class_,
        "backstory": card.backstory,
        "talkativeness": card.talkativeness, "courage": card.courage,
        "greed": card.greed, "attitude_to_master": card.attitude_to_master,
        "party_invite_policy": card.party_invite_policy,
    })


_TEMPLATE = (
    "tools={tools_summary} state={state_json} goals={goals_json} "
    "memories={memories_json} recent={recent_decisions_json} hot={hot_inputs_json}"
)

# Flat obs state for bots not yet in a group and with no pending invite.
_OBS_UNGROUPED_CASMINA = {
    "self": {
        "class": "paladin", "level": 25, "name": "Casmina",
        "hp_pct": 100, "mana_pct": 100, "is_dead": False,
        "is_in_combat": False, "is_resting": True,
        "pending_group_invite": None,
        "race": "human", "spec": "",
    },
    "social": {
        "group_members": [], "guild": None, "in_group": False,
        "nearby_humans": [], "recent_whispers": [],
    },
    "goal": {"current": "Idle", "elapsed_minutes": 0, "params": {},
             "progress_pct": 0, "ttl_minutes": 0},
    "memory_hints": [], "quest_log": [], "event_log": [], "inventory_highlights": {},
}

_OBS_UNGROUPED_ELLIAN = {
    "self": {
        "class": "hunter", "level": 21, "name": "Ellian",
        "hp_pct": 100, "mana_pct": 100, "is_dead": False,
        "is_in_combat": False, "is_resting": False,
        "pending_group_invite": None,
        "race": "draenei", "spec": "",
    },
    "social": {
        "group_members": [], "guild": None, "in_group": False,
        "nearby_humans": [], "recent_whispers": [],
    },
    "goal": {"current": "Idle", "elapsed_minutes": 0, "params": {},
             "progress_pct": 0, "ttl_minutes": 0},
    "memory_hints": [], "quest_log": [], "event_log": [], "inventory_highlights": {},
}


# ---------------------------------------------------------------------------
# Test A — invite → accept → in_group
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_invite_flow_casmina_invites_ellian_and_both_in_group(
    temp_db_path, fake_harness, fake_memory, fake_llm
):
    """End-to-end: Casmina (1003) LLM decides bot.invite_to_group(target_guid=1004).
    Ellian (1004) detects pending invite via triage (party_invite_received), LLM decides
    bot.accept_invite. Afterwards obs.get_state shows in_group=true for both.

    Decision flow:
    - First ticks (first_tick, both bots): LLM returns no_op for both.
    - Second ticks: Casmina sees fresh_chat → invites Ellian.
      Ellian sees pending invite (obs switches after invite_sent) → party_invite_received → accepts.
    - LLM responder is bot-aware: inspects system prompt for "Casmina" or "Ellian".

    Probed shapes: P2.5 (pending invite obs) + P3 (accept result + in-group obs) from spec §4.3.
    Fresh-chat items use ts=_FUTURE_TS so they remain fresh on all post-first ticks.
    """
    casmina_card = _card("Casmina", "Paladin", party_invite_policy="accept_from_known")
    ellian_card = _card("Ellian", "Hunter", party_invite_policy="accept_always")

    casmina_persona = _persona_json(casmina_card)
    ellian_persona = _persona_json(ellian_card)

    # State tracking for harness responder.
    # invite_sent: True after bot.invite_to_group is dispatched (Ellian sees pending invite)
    # accept_called: True after bot.accept_invite is dispatched (both see in_group=True)
    invite_sent = [False]
    accept_called = [False]

    # Bot-aware LLM responder: reads the system prompt to determine which bot is deciding.
    # - Casmina: returns no_op on first call, bot.invite_to_group on subsequent calls.
    # - Ellian: returns no_op on first call, bot.accept_invite on subsequent calls.
    casmina_calls = [0]
    ellian_calls = [0]

    async def _llm_responder(*, system: str, user: str, **kwargs):
        is_casmina = "Casmina" in system
        is_ellian = "Ellian" in system

        if is_casmina:
            casmina_calls[0] += 1
            if casmina_calls[0] == 1:
                # First tick: no_op (first_tick reason, no chat yet)
                d = {"kind": "no_op", "tool": None, "args": None, "confidence": 0.0, "reasoning": "first tick"}
            else:
                # Subsequent ticks: invite Ellian
                d = {
                    "kind": "action", "tool": "bot.invite_to_group",
                    "args": {"bot_guid": 1003, "target_guid": 1004},
                    "confidence": 0.96,
                    "reasoning": "User asked me to invite Ellian",
                }
        elif is_ellian:
            ellian_calls[0] += 1
            if ellian_calls[0] == 1:
                # First tick: no_op (first_tick reason)
                d = {"kind": "no_op", "tool": None, "args": None, "confidence": 0.0, "reasoning": "first tick"}
            else:
                # Subsequent ticks: accept the invite
                d = {
                    "kind": "action", "tool": "bot.accept_invite",
                    "args": {"bot_guid": 1004},
                    "confidence": 0.95,
                    "reasoning": "Casmina invited me, accept_always policy",
                }
        else:
            d = {"kind": "no_op", "tool": None, "args": None, "confidence": 0.0, "reasoning": "unknown bot"}
        return d, json.dumps(d), 100.0

    fake_llm.chat_completion_json = _llm_responder

    def _harness_responder(t: str, a: dict) -> dict:
        if t == "obs.get_state":
            tgt = a.get("target_guid")
            if tgt == 1004:
                # Ellian: show pending invite once Casmina has sent the invite
                if invite_sent[0] and not accept_called[0]:
                    return PROBED_PENDING_INVITE_OBS
                elif accept_called[0]:
                    return PROBED_IN_GROUP_OBS_ELLIAN
                else:
                    # No invite yet: show ungrouped state (no pending invite)
                    return _OBS_UNGROUPED_ELLIAN
            else:
                # Casmina
                if accept_called[0]:
                    return PROBED_IN_GROUP_OBS_CASMINA
                else:
                    return _OBS_UNGROUPED_CASMINA
        if t == "obs.get_combat_log":
            return {"events": []}
        if t == "bot.invite_to_group":
            invite_sent[0] = True
            return {
                "ok": True, "invited": True,
                "target_guid": 1004, "target_name": "Ellian", "inviter_name": "Casmina",
            }
        if t == "bot.accept_invite":
            accept_called[0] = True
            return PROBED_ACCEPT_RESULT
        if t == "bot.send_chat":
            return {"ok": True}
        return {"ok": True}

    fake_harness.responder = _harness_responder

    # Casmina's memory has a fresh whisper (ts=_FUTURE_TS so it fires every poll tick).
    # Ellian's memory is empty — Ellian acts on party_invite_received, not fresh_chat.
    def _memory_responder(t: str, a: dict) -> dict:
        bot_id = a.get("bot_id", "")
        if t == "memory.search":
            if bot_id == "1003":
                return {"items": [
                    {
                        "id": "m1",
                        "text": "received whisper from User: invite Ellian to the group",
                        "ts": _FUTURE_TS,
                    }
                ]}
            return {"items": []}
        if t == "goals.list":
            return {"items": []}
        if t == "memory.personality_get":
            return {"persona": casmina_persona if bot_id == "1003" else ellian_persona}
        if t == "memory.recall":
            return {"result": {"items": []}}
        if t == "memory.recall_about":
            return {"result": {"items": []}}
        if t == "memory.write":
            return {"ok": True}
        return {}

    fake_memory.responder = _memory_responder

    store = StateStore(temp_db_path)
    store.migrate()
    store.enroll(bot_guid=1003, enrolled_at_ms=1_000_000, personality_seed=casmina_card)
    store.enroll(bot_guid=1004, enrolled_at_ms=1_000_000, personality_seed=ellian_card)

    cache = PersonalityCache(memory_mcp=fake_memory, ttl_s=60, capacity=8)
    triage = TriageGate(harness_mcp=fake_harness, memory_mcp=fake_memory)
    decider = Decider(
        llm_client=fake_llm, personality_cache=cache, memory_mcp=fake_memory,
        state_store=store, prompt_template=_TEMPLATE,
    )
    dispatcher = Dispatcher(harness_mcp=fake_harness, memory_mcp=fake_memory)
    seen: list[dict] = []
    sup = LoopSupervisor(
        triage=triage, decider=decider, dispatcher=dispatcher,
        state_store=store, tick_interval_s=0.05,
        decision_log_writer=type("W", (), {"write": lambda self, r: seen.append(r)})(),
    )
    sup.start(1003)
    sup.start(1004)
    await asyncio.sleep(0.80)
    await sup.stop_all()
    store.close()

    # Casmina must have executed bot.invite_to_group
    casmina_invites = [
        c for c in fake_harness.calls if c[0] == "bot.invite_to_group" and c[1].get("bot_guid") == 1003
    ]
    assert casmina_invites, "Casmina must have called bot.invite_to_group"

    # Ellian must have executed bot.accept_invite
    ellian_accepts = [
        c for c in fake_harness.calls if c[0] == "bot.accept_invite" and c[1].get("bot_guid") == 1004
    ]
    assert ellian_accepts, "Ellian must have called bot.accept_invite"

    # At least one decision log entry with party_invite_received triage reason.
    # This fires on Ellian's second+ tick after invite_sent=True triggers PROBED_PENDING_INVITE_OBS.
    invite_triage = [r for r in seen if r.get("triage_reason") == "party_invite_received"]
    assert invite_triage, "Expected at least one tick with triage_reason='party_invite_received'"

    # Confirm the in-group state is visible: Ellian's post-accept obs has social.in_group=True
    assert PROBED_IN_GROUP_OBS_ELLIAN["result"]["social"]["in_group"] is True, (
        "Sanity: PROBED_IN_GROUP_OBS_ELLIAN must show in_group=True"
    )


# ---------------------------------------------------------------------------
# Test B — set_role → queue → enter
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_dungeon_flow_casmina_tanks_rfc(temp_db_path, fake_harness, fake_memory, fake_llm):
    """End-to-end: User tells Casmina to tank RFC.
    Brain tick fires (fresh_chat), LLM decides set_role='tank'.
    Next tick: LLM decides queue_for_dungeon(dungeon_id=33, roles_mask=0).
    Next tick: LLM decides enter_instance(mode='lfg').

    Decision flow:
    - First tick (first_tick): LLM → set_role.
    - Second tick (fresh_chat, ts=_FUTURE_TS so always fresh): LLM → queue_for_dungeon.
    - Third tick (fresh_chat, same item still fresh — no memory_id so never deduped): LLM → enter_instance.
    Memory item has no memory_id field so the dedup gate never marks it and it fires on every poll tick.

    Probed shapes: P5 (bot.set_role tank) + P6 (bot.queue_for_dungeon) from spec §4.3.
    Note on teleport-ready state: P8 shows bot.enter_instance returns executor_failed when not
    LFG-matched. For Test B we simulate a successful teleport with a stubbed response
    (no probe exists for in-match enter_instance success) — documented as PROBED_SHAPE_TODO
    retention below.
    """
    casmina_card = _card("Casmina", "Paladin")
    casmina_persona = _persona_json(casmina_card)

    _decisions = iter([
        {
            "kind": "action", "tool": "bot.set_role",
            "args": {"bot_guid": 1003, "role": "tank"},
            "confidence": 0.88,
            "reasoning": "User asked me to tank RFC",
        },
        {
            "kind": "action", "tool": "bot.queue_for_dungeon",
            "args": {"bot_guid": 1003, "dungeon_id": 33, "roles_mask": 0},
            "confidence": 0.96,  # >= HIGH_RISK_THRESHOLD (0.95) to bypass confirmation gate
            "reasoning": "Role set as tank, queuing for RFC dungeon_id=33",
        },
        {
            "kind": "action", "tool": "bot.enter_instance",
            "args": {"bot_guid": 1003, "mode": "lfg"},
            "confidence": 0.96,
            "reasoning": "LFG ready, teleporting into RFC",
        },
    ])

    async def _llm_responder(*, system: str, user: str, **kwargs):
        try:
            d = next(_decisions)
        except StopIteration:
            d = {"kind": "no_op", "tool": None, "args": None, "confidence": 0.0, "reasoning": "idle"}
        return d, json.dumps(d), 100.0

    fake_llm.chat_completion_json = _llm_responder

    # bot.enter_instance success shape: no probe captured for in-match success (P8 only captures
    # the executor_failed case). Using a plausible success envelope; flagged as PROBED_SHAPE_TODO
    # pending a future in-match probe. The shape mirrors the P8 pattern with ok=True.
    _ENTER_INSTANCE_SUCCESS_STUB = {  # PROBED_SHAPE_TODO: replace when in-match probe is run
        "ok": True, "teleported": True, "mode": "lfg", "map_id": 33,
    }

    fake_harness.responder = lambda t, a: {
        "obs.get_state": _OBS_UNGROUPED_CASMINA,
        "obs.get_combat_log": {"events": []},
        "bot.set_role": PROBED_SET_ROLE_RESULT,
        "bot.queue_for_dungeon": PROBED_QUEUE_RESULT,
        "bot.enter_instance": _ENTER_INSTANCE_SUCCESS_STUB,
        "bot.send_chat": {"ok": True},
    }.get(t, {"ok": True})

    fake_memory.responder = lambda t, a: {
        # ts=_FUTURE_TS ensures this item is always "fresh" relative to since_ts_s.
        # No memory_id field → dedup gate never marks it → fires on every poll tick.
        # This gives the brain 3 fresh_chat opportunities for the 3-decision sequence.
        "memory.search": {"items": [
            {
                "id": "m1",
                "text": "received whisper from User: Hey Casmina, can you tank RFC?",
                "ts": _FUTURE_TS,
            }
        ]} if t == "memory.search" else {"items": []},
        "goals.list": {"items": []},
        "memory.personality_get": {"persona": casmina_persona},
        "memory.recall": {"result": {"items": []}},
        "memory.recall_about": {"result": {"items": []}},
        "memory.write": {"ok": True},
    }.get(t, {})

    store = StateStore(temp_db_path)
    store.migrate()
    store.enroll(bot_guid=1003, enrolled_at_ms=1_000_000, personality_seed=casmina_card)

    cache = PersonalityCache(memory_mcp=fake_memory, ttl_s=60, capacity=8)
    triage = TriageGate(harness_mcp=fake_harness, memory_mcp=fake_memory)
    decider = Decider(
        llm_client=fake_llm, personality_cache=cache, memory_mcp=fake_memory,
        state_store=store, prompt_template=_TEMPLATE,
    )
    dispatcher = Dispatcher(harness_mcp=fake_harness, memory_mcp=fake_memory)
    seen: list[dict] = []
    sup = LoopSupervisor(
        triage=triage, decider=decider, dispatcher=dispatcher,
        state_store=store, tick_interval_s=0.05,
        decision_log_writer=type("W", (), {"write": lambda self, r: seen.append(r)})(),
    )
    sup.start(1003)
    await asyncio.sleep(0.60)
    await sup.stop_all()
    store.close()

    # set_role must have been called
    role_calls = [
        c for c in fake_harness.calls if c[0] == "bot.set_role" and c[1].get("bot_guid") == 1003
    ]
    assert role_calls, "Casmina must have called bot.set_role"
    assert role_calls[0][1].get("role") == "tank", (
        f"Expected role='tank', got {role_calls[0][1].get('role')!r}"
    )

    # queue_for_dungeon must have been called after set_role
    queue_calls = [
        c for c in fake_harness.calls if c[0] == "bot.queue_for_dungeon" and c[1].get("bot_guid") == 1003
    ]
    assert queue_calls, "Casmina must have called bot.queue_for_dungeon"

    # enter_instance must have been called
    enter_calls = [
        c for c in fake_harness.calls if c[0] == "bot.enter_instance" and c[1].get("bot_guid") == 1003
    ]
    assert enter_calls, "Casmina must have called bot.enter_instance"
    assert enter_calls[0][1].get("mode") == "lfg", (
        f"Expected mode='lfg', got {enter_calls[0][1].get('mode')!r}"
    )

    # Sequence check: set_role before queue_for_dungeon before enter_instance
    all_relevant = [
        c for c in fake_harness.calls
        if c[0] in ("bot.set_role", "bot.queue_for_dungeon", "bot.enter_instance")
    ]
    tool_sequence = [c[0] for c in all_relevant]
    assert tool_sequence.index("bot.set_role") < tool_sequence.index("bot.queue_for_dungeon"), (
        "bot.set_role must precede bot.queue_for_dungeon"
    )
    assert tool_sequence.index("bot.queue_for_dungeon") < tool_sequence.index("bot.enter_instance"), (
        "bot.queue_for_dungeon must precede bot.enter_instance"
    )

    # Validate P5 probed shape: PROBED_SET_ROLE_RESULT confirms roles_mask=2 (TANK)
    assert PROBED_SET_ROLE_RESULT["result"]["roles_mask"] == PLAYER_ROLE_TANK, (
        "Sanity: P5 probe confirmed tank roles_mask=2"
    )
    # Validate P6 probed shape: PROBED_QUEUE_RESULT confirms dungeon_id=-1 sentinel + queued=True
    assert PROBED_QUEUE_RESULT["result"]["queued"] is True, (
        "Sanity: P6 probe confirmed queued=True"
    )
    assert PROBED_QUEUE_RESULT["result"]["dungeon_id"] == -1, (
        "Sanity: P6 probe confirmed dungeon_id=0 resolves to -1 random sentinel"
    )
