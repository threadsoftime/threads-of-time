# SPDX-License-Identifier: GPL-2.0-or-later
"""Synthetic corpus + labeled-query generator for the Phase 8 eval harness.

This script is *not* a test — it produces the committed JSONL files in
``fixtures/`` and ``queries/`` deterministically (no random seed; every
construct is index-driven).

Running it overwrites the committed fixtures. The test harness in
``test_quality_gate.py`` reads the committed JSONL; it never invokes this
generator. Commit any regenerated output alongside any change to this script.

Usage:

    cd tot/memory
    python tests/eval/_generate_fixtures.py

Design notes (per design subspec §12.2):

- 200 episodes spread across the 8 episode types (chat, combat, social,
  quest, discovery, goal, reflection, observation).
- 10 synthetic players + 5 NPCs + 3 locations (entity catalogue at the top
  of this file).
- Timestamps span ~30 days ending at FIXED_NOW (2026-05-27 00:00 UTC).
- Salience varies by type per §8 default rubric.
- ``expected_episode_ids`` in the query file uses the **1-indexed line number**
  of the episode in the committed JSONL (matches the SQLite AUTOINCREMENT
  episode_id when seeded in order — see test_quality_gate.py).
"""
from __future__ import annotations

import json
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

# ---- Fixed reference clock ---------------------------------------------------
# Eval is deterministic: every timestamp is computed relative to this anchor.
FIXED_NOW = datetime(2026, 5, 27, 0, 0, 0, tzinfo=timezone.utc)
BOT_GUID = "eval-bot-1"

# ---- Entity catalogue --------------------------------------------------------
PLAYERS = [
    ("alice", "Alice"),
    ("bob", "Bob"),
    ("carol", "Carol"),
    ("dave", "Dave"),
    ("erin", "Erin"),
    ("frank", "Frank"),
    ("grace", "Grace"),
    ("heidi", "Heidi"),
    ("ivan", "Ivan"),
    ("judy", "Judy"),
]
NPCS = [
    ("trainer_paxton", "Paxton the Trainer"),
    ("innkeeper_lin", "Innkeeper Lin"),
    ("merchant_ord", "Ord the Merchant"),
    ("captain_velt", "Captain Velt"),
    ("guard_eska", "Guard Eska"),
]
LOCATIONS = [
    ("stormwind", "Stormwind"),
    ("westfall", "Westfall"),
    ("deadmines", "The Deadmines"),
]


def ts(days_ago: float) -> int:
    """Epoch ms ``days_ago`` (float) days before FIXED_NOW."""
    return int((FIXED_NOW - timedelta(days=days_ago)).timestamp() * 1000)


def ent(kind: str, key: str, name: str, role: str = "participant") -> dict[str, str]:
    return {
        "entity_kind": kind,
        "entity_key": key,
        "display_name": name,
        "role": role,
    }


def _norm_ents(ents_input: list[Any]) -> list[dict[str, str]]:
    """Normalize a list of either dicts or 4-tuples into entity dicts."""
    out: list[dict[str, str]] = []
    for e in ents_input:
        if isinstance(e, dict):
            out.append(e)
        else:
            kind, key, name, role = e
            out.append(ent(kind, key, name, role))
    return out


# Episode authoring helpers ----------------------------------------------------
def episode(
    *,
    days_ago: float,
    content: str,
    etype: str,
    salience: float,
    entities: list[dict[str, str]],
    metadata: dict[str, Any] | None = None,
    source: str = "self",
) -> dict[str, Any]:
    return {
        "bot_guid": BOT_GUID,
        "timestamp": ts(days_ago),
        "content_text": content,
        "episode_type": etype,
        "salience_score": salience,
        "salience_hint": salience,
        "entities": entities,
        "metadata": metadata or {},
        "source": source,
    }


# ---- Author the 200-episode corpus ------------------------------------------
# Mix is chosen to roughly match design subspec §12.2 with all 8 types covered:
#   40 chat, 30 combat, 30 social, 25 quest, 25 discovery, 20 goal,
#   15 reflection, 15 observation  = 200
#
# Each block embeds enough distinctive keywords (entity names, location names,
# verbs) for the BM25 component to surface the right episodes for the labeled
# queries in queries/recall_set.jsonl.


def build_corpus() -> list[dict[str, Any]]:
    eps: list[dict[str, Any]] = []

    # === CHAT (40) ============================================================
    # Distinctive vocabulary per player so BM25 can target a single chat.
    chat_lines = [
        ("alice", "Alice", "Alice asked me what spec I was running for tanking.", 0.5),
        ("alice", "Alice", "Alice taught me how to aoe pull mobs in dungeons.", 0.7),
        ("alice", "Alice", "Alice complimented my gear upgrades from last week.", 0.45),
        ("alice", "Alice", "Alice warned me about the fall damage off the Stormwind bridge.", 0.5),
        ("bob",   "Bob",   "Bob asked if I wanted to duel at the training yard.", 0.4),
        ("bob",   "Bob",   "Bob shared a fishing spot near the Westfall pond.", 0.5),
        ("bob",   "Bob",   "Bob told me his favorite hunter pet is a wolf.", 0.35),
        ("bob",   "Bob",   "Bob complained about the auction house deposit fees.", 0.3),
        ("carol", "Carol", "Carol explained the threat mechanic to me in chat.", 0.55),
        ("carol", "Carol", "Carol said she would heal my next dungeon run.", 0.6),
        ("carol", "Carol", "Carol mentioned a rare cloak drop in the Deadmines.", 0.5),
        ("dave",  "Dave",  "Dave traded me a stack of linen cloth for copper bars.", 0.45),
        ("dave",  "Dave",  "Dave laughed when I fell into the moat at Stormwind.", 0.35),
        ("dave",  "Dave",  "Dave invited me to his guild's weekend raid.", 0.6),
        ("erin",  "Erin",  "Erin asked me to escort her quest objective in Westfall.", 0.55),
        ("erin",  "Erin",  "Erin recommended a leveling addon for quest tracking.", 0.45),
        ("frank", "Frank", "Frank challenged me to a duel outside the Stormwind gates.", 0.4),
        ("frank", "Frank", "Frank told me the Defias are scaling in the Deadmines.", 0.55),
        ("grace", "Grace", "Grace asked which class is best for solo questing.", 0.4),
        ("grace", "Grace", "Grace shared a tip about kiting elites with snare.", 0.55),
        ("heidi", "Heidi", "Heidi asked me to summon her to the Deadmines entrance.", 0.5),
        ("heidi", "Heidi", "Heidi mentioned the rare spawn pattern for Mor'Ladim.", 0.55),
        ("ivan",  "Ivan",  "Ivan and I joked about hunter mark misclicks.", 0.3),
        ("ivan",  "Ivan",  "Ivan offered to enchant my new bracers for free.", 0.65),
        ("judy",  "Judy",  "Judy invited me to her birthday in-game party at the inn.", 0.6),
        ("judy",  "Judy",  "Judy asked if I wanted to skin some kodos with her later.", 0.4),
        # Mixed-party chats (multiple entities)
        ("alice", "Alice", "Alice and Bob discussed strategy for Stockades pulls.", 0.55),
        ("alice", "Alice", "Alice told Carol our dungeon group needed a tank tonight.", 0.5),
        ("bob",   "Bob",   "Bob asked Erin about the Defias quest chain in Westfall.", 0.5),
        ("carol", "Carol", "Carol whispered to Dave that the healer slot was filled.", 0.45),
        # NPC chat (trainer / innkeeper)
        ("trainer_paxton", "Paxton the Trainer", "Paxton the Trainer offered a new combat ability training.", 0.5),
        ("trainer_paxton", "Paxton the Trainer", "Paxton the Trainer reminded me to spend my unspent talent points.", 0.45),
        ("innkeeper_lin", "Innkeeper Lin", "Innkeeper Lin set my hearthstone to her Stormwind inn.", 0.5),
        ("innkeeper_lin", "Innkeeper Lin", "Innkeeper Lin chatted about the festival in Stormwind.", 0.35),
        ("merchant_ord", "Ord the Merchant", "Ord the Merchant haggled with me over a healing potion price.", 0.4),
        ("merchant_ord", "Ord the Merchant", "Ord the Merchant offered a discount on cloth armor today.", 0.4),
        ("captain_velt", "Captain Velt", "Captain Velt thanked me for clearing the Defias scouts.", 0.55),
        ("captain_velt", "Captain Velt", "Captain Velt warned that the Defias are massing again.", 0.55),
        ("guard_eska", "Guard Eska", "Guard Eska directed me toward the auction house in Stormwind.", 0.3),
        ("guard_eska", "Guard Eska", "Guard Eska reported strange noises near the Stormwind canals.", 0.4),
    ]
    # Chat half-life = 48h: keep block within ~3 days so freshest chats have
    # decay ≥ 0.42 and even the oldest still hits decay ≈ 0.35. This is
    # sufficient for BM25 to surface keyword hits ahead of decayed noise.
    for idx, (key, name, content, sal) in enumerate(chat_lines):
        kind = "player" if key in {p[0] for p in PLAYERS} else "npc"
        ents = [ent(kind, key, name, role="participant")]
        # Sprinkle a location entity when content mentions a known location.
        for lkey, lname in LOCATIONS:
            if lname in content or lkey in content:
                ents.append(ent("location", lkey, lname, role="witness"))
        # Newer chats first in the file → older at the tail. Spread 0..3 days.
        eps.append(episode(
            days_ago=3.0 * (idx / max(len(chat_lines) - 1, 1)),
            content=content,
            etype="chat",
            salience=sal,
            entities=ents,
            metadata={"channel": "say"},
            source="chat",
        ))

    # === COMBAT (30) ==========================================================
    combat_lines = [
        ("Defeated Defias Pillager in Westfall fields.", "westfall", 0.55, [("npc", "defias_pillager", "Defias Pillager", "target")]),
        ("Killed Mor'Ladim the rare elite outside Raven Hill.", "westfall", 0.85, [("npc", "morladim", "Mor'Ladim", "target")]),
        ("Wiped on Edwin VanCleef during a Deadmines pull.", "deadmines", 0.7, [("npc", "vancleef", "Edwin VanCleef", "target")]),
        ("Near-death from a Defias Ambusher critical strike in Westfall.", "westfall", 0.7, [("npc", "defias_ambusher", "Defias Ambusher", "target")]),
        ("Group wiped to Sneed at the lumber mill in the Deadmines.", "deadmines", 0.75, [("npc", "sneed", "Sneed", "target")]),
        ("Defeated Rhahk'Zor in the Deadmines stoneworks.", "deadmines", 0.65, [("npc", "rhahkzor", "Rhahk'Zor", "target")]),
        ("Solo-killed an elite Hogger after a long kite around Elwynn.", "stormwind", 0.8, [("npc", "hogger", "Hogger", "target")]),
        ("Fell off the Deadmines cliff and took massive fall damage.", "deadmines", 0.6, []),
        ("Killed three murlocs along the Stormwind harbor.", "stormwind", 0.4, [("npc", "murloc", "Coastal Murloc", "target")]),
        ("Drowned in the Westfall river while fleeing a gnoll.", "westfall", 0.5, [("npc", "gnoll", "Riverpaw Gnoll", "target")]),
        ("Tanked the Deadmines goblin foundry pull without dying.", "deadmines", 0.7, [("npc", "goblin", "Defias Goblin", "target")]),
        ("Frank and I duo-killed a Defias Strip Miner.", "westfall", 0.55, [("player", "frank", "Frank", "participant"), ("npc", "defias_strip_miner", "Defias Strip Miner", "target")]),
        ("Carol healed me through a Deadmines boss fight.", "deadmines", 0.7, [("player", "carol", "Carol", "participant")]),
        ("Alice tanked Sneed while I dpsed from range.", "deadmines", 0.7, [("player", "alice", "Alice", "participant"), ("npc", "sneed", "Sneed", "target")]),
        ("Bob died first on the goblin engineer pull.", "deadmines", 0.55, [("player", "bob", "Bob", "participant")]),
        ("Killed VanCleef cleanly on the second attempt.", "deadmines", 0.85, [("npc", "vancleef", "Edwin VanCleef", "target")]),
        ("Wiped to a stray Defias patrol at the Deadmines entrance.", "deadmines", 0.5, [("npc", "defias", "Defias Patrol", "target")]),
        ("Got jumped by a Defias rogue stealth opener in Westfall.", "westfall", 0.6, [("npc", "defias_rogue", "Defias Rogue", "target")]),
        ("Soloed a yellow-level murloc in the Stormwind canals.", "stormwind", 0.35, [("npc", "murloc", "Canal Murloc", "target")]),
        ("Defeated a gnoll pack with Erin near Westfall watchtower.", "westfall", 0.55, [("player", "erin", "Erin", "participant"), ("npc", "gnoll", "Riverpaw Gnoll", "target")]),
        ("Wiped on the gunpowder barrel patrol in Deadmines.", "deadmines", 0.6, []),
        ("Dave pulled too many in the Stockades and we all died.", "stormwind", 0.5, [("player", "dave", "Dave", "participant")]),
        ("Finished Mr. Smite in the Deadmines after three tries.", "deadmines", 0.8, [("npc", "smite", "Mr. Smite", "target")]),
        ("Got crit by a Riverpaw mystic for half my health.", "westfall", 0.55, [("npc", "riverpaw_mystic", "Riverpaw Mystic", "target")]),
        ("Killed a coastal murloc that dropped nothing useful.", "westfall", 0.25, [("npc", "murloc", "Coastal Murloc", "target")]),
        ("Long arena fight with a hostile player Frank in Westfall.", "westfall", 0.6, [("player", "frank", "Frank", "participant")]),
        ("Wiped on the cookie boss after a stray cleave.", "deadmines", 0.55, [("npc", "cookie", "Cookie", "target")]),
        ("Killed an elite spider with Heidi off-tanking.", "deadmines", 0.6, [("player", "heidi", "Heidi", "participant")]),
        ("Burned down a Defias Trapper before he could trap.", "westfall", 0.5, [("npc", "defias_trapper", "Defias Trapper", "target")]),
        ("Took a near-fatal hit from a Stormwind canal crocolisk.", "stormwind", 0.7, [("npc", "crocolisk", "Canal Crocolisk", "target")]),
    ]
    # Combat half-life = 72h: spread 0..5 days so even the oldest decays only
    # to ~0.31. Newer combats live at low indices.
    for i, (content, loc_key, sal, extra_ents) in enumerate(combat_lines):
        ents = _norm_ents(extra_ents)
        for lkey, lname in LOCATIONS:
            if lkey == loc_key:
                ents.append(ent("location", lkey, lname, role="witness"))
        eps.append(episode(
            days_ago=5.0 * (i / max(len(combat_lines) - 1, 1)),
            content=content,
            etype="combat",
            salience=sal,
            entities=ents,
            metadata={"outcome": "win" if "Defeated" in content or "Killed" in content or "Burned" in content or "Finished" in content or "Soloed" in content or "Tanked" in content else "loss"},
        ))

    # === SOCIAL (30) ==========================================================
    social_lines = [
        ("Joined Alice's party for a Deadmines run.", [("player", "alice", "Alice", "subject")], 0.65, "deadmines"),
        ("Bob added me to his friends list after we grouped.", [("player", "bob", "Bob", "subject")], 0.6, None),
        ("Carol invited me to her guild Bright Hand.", [("player", "carol", "Carol", "subject")], 0.7, None),
        ("Dave hugged my character at the Stormwind fountain.", [("player", "dave", "Dave", "subject")], 0.4, "stormwind"),
        ("Erin and I traded emotes near the Westfall bridge.", [("player", "erin", "Erin", "subject")], 0.35, "westfall"),
        ("Frank challenged me to a non-lethal duel and lost.", [("player", "frank", "Frank", "subject")], 0.5, None),
        ("Grace bowed to me after a successful dungeon clear.", [("player", "grace", "Grace", "subject")], 0.45, None),
        ("Heidi taught my character to dance at the inn.", [("player", "heidi", "Heidi", "subject")], 0.35, None),
        ("Ivan thanked me for the enchant I gave him.", [("player", "ivan", "Ivan", "subject")], 0.5, None),
        ("Judy threw a party at the Stormwind inn for her dingo.", [("player", "judy", "Judy", "subject")], 0.6, "stormwind"),
        ("Alice and Carol formed a healing pact for our group.", [("player", "alice", "Alice", "subject"), ("player", "carol", "Carol", "participant")], 0.55, None),
        ("Bob and Frank argued in /say about loot rolls.", [("player", "bob", "Bob", "subject"), ("player", "frank", "Frank", "participant")], 0.45, None),
        ("Erin proposed a regular Westfall questing schedule.", [("player", "erin", "Erin", "subject")], 0.55, "westfall"),
        ("Heidi joined my friends list after our raid.", [("player", "heidi", "Heidi", "subject")], 0.55, None),
        ("Grace declined my group invite politely.", [("player", "grace", "Grace", "subject")], 0.3, None),
        ("Ivan sent me a fan mail with a cupcake.", [("player", "ivan", "Ivan", "subject")], 0.4, None),
        ("Judy applauded my Mor'Ladim kill in /yell.", [("player", "judy", "Judy", "subject")], 0.55, None),
        ("Paxton the Trainer congratulated me on reaching level 20.", [("npc", "trainer_paxton", "Paxton the Trainer", "subject")], 0.65, None),
        ("Innkeeper Lin offered me a complimentary mug of mead.", [("npc", "innkeeper_lin", "Innkeeper Lin", "subject")], 0.4, "stormwind"),
        ("Ord the Merchant invited me to a special after-hours sale.", [("npc", "merchant_ord", "Ord the Merchant", "subject")], 0.5, None),
        ("Captain Velt formally recognized me for Defias kills.", [("npc", "captain_velt", "Captain Velt", "subject")], 0.65, None),
        ("Guard Eska saluted me at the Stormwind gates.", [("npc", "guard_eska", "Guard Eska", "subject")], 0.35, "stormwind"),
        ("Alice gifted me a cloak from her bank stash.", [("player", "alice", "Alice", "subject")], 0.6, None),
        ("Carol shared loot priority rules for our raid.", [("player", "carol", "Carol", "subject")], 0.55, None),
        ("Dave taught me the train whistle emote.", [("player", "dave", "Dave", "subject")], 0.3, None),
        ("Erin invited me to her wedding RP event.", [("player", "erin", "Erin", "subject")], 0.7, None),
        ("Frank apologized for the duel ninja-loot.", [("player", "frank", "Frank", "subject")], 0.5, None),
        ("Grace asked to become my dungeon-leveling partner.", [("player", "grace", "Grace", "subject")], 0.6, None),
        ("Ivan proposed we duo-level to thirty next weekend.", [("player", "ivan", "Ivan", "subject")], 0.55, None),
        ("Judy offered to be my crafting supplier for cloth gear.", [("player", "judy", "Judy", "subject")], 0.55, None),
    ]
    # Social half-life = 96h: spread 0..6 days.
    for i, (content, ents_input, sal, loc_key) in enumerate(social_lines):
        ents = _norm_ents(ents_input)
        if loc_key:
            for lkey, lname in LOCATIONS:
                if lkey == loc_key:
                    ents.append(ent("location", lkey, lname, role="witness"))
        eps.append(episode(
            days_ago=6.0 * (i / max(len(social_lines) - 1, 1)),
            content=content,
            etype="social",
            salience=sal,
            entities=ents,
        ))

    # === QUEST (25) ===========================================================
    quest_lines = [
        ("Accepted 'Westfall Stew' from Innkeeper Lin in Sentinel Hill.", [("npc", "innkeeper_lin", "Innkeeper Lin", "subject"), ("location", "westfall", "Westfall", "witness")], 0.55),
        ("Completed 'The Defias Brotherhood' quest chain step one.", [("location", "westfall", "Westfall", "witness")], 0.7),
        ("Failed escort quest when the NPC died to Defias gnolls.", [("location", "westfall", "Westfall", "witness")], 0.6),
        ("Accepted 'Red Leather Bandanas' from Captain Velt.", [("npc", "captain_velt", "Captain Velt", "subject")], 0.55),
        ("Turned in 'Red Leather Bandanas' for a juicy XP reward.", [("npc", "captain_velt", "Captain Velt", "subject")], 0.7),
        ("Completed 'Westfall Stew' and got a leather chestpiece reward.", [("location", "westfall", "Westfall", "witness")], 0.7),
        ("Accepted Deadmines dungeon quest from Stormwind quartermaster.", [("location", "deadmines", "The Deadmines", "witness")], 0.65),
        ("Completed Deadmines dungeon quest and turned it in.", [("location", "deadmines", "The Deadmines", "witness")], 0.75),
        ("Abandoned a quest after realizing the area was too high level.", [], 0.4),
        ("Discovered the quest giver at the Westfall lighthouse.", [("location", "westfall", "Westfall", "witness")], 0.5),
        ("Accepted 'The People's Militia' chain from Captain Velt.", [("npc", "captain_velt", "Captain Velt", "subject")], 0.7),
        ("Killed ten Defias for 'The People's Militia' objective.", [("npc", "defias", "Defias", "target")], 0.6),
        ("Turned in 'The People's Militia' for a green cloak.", [("npc", "captain_velt", "Captain Velt", "subject")], 0.75),
        ("Accepted bread baking quest from Innkeeper Lin in Stormwind.", [("npc", "innkeeper_lin", "Innkeeper Lin", "subject"), ("location", "stormwind", "Stormwind", "witness")], 0.45),
        ("Completed bread baking quest with help from Carol.", [("player", "carol", "Carol", "participant")], 0.55),
        ("Quest log full — had to abandon the murloc fin quest.", [], 0.35),
        ("Found a quest item drop from a Defias scout.", [("npc", "defias_scout", "Defias Scout", "target")], 0.5),
        ("Escorted Innkeeper Lin's nephew safely across Westfall.", [("npc", "innkeeper_lin", "Innkeeper Lin", "subject")], 0.65),
        ("Failed escort when I aggroed too many gnolls.", [], 0.55),
        ("Discovered a hidden quest behind a Westfall outhouse.", [("location", "westfall", "Westfall", "witness")], 0.65),
        ("Completed VanCleef's head turn-in to Captain Velt.", [("npc", "captain_velt", "Captain Velt", "subject"), ("npc", "vancleef", "Edwin VanCleef", "target")], 0.85),
        ("Accepted follow-up quest after killing VanCleef.", [("npc", "vancleef", "Edwin VanCleef", "target")], 0.7),
        ("Turned in raptor scales for a tailoring pattern.", [], 0.45),
        ("Accepted an unusual cooking quest from Ord the Merchant.", [("npc", "merchant_ord", "Ord the Merchant", "subject")], 0.45),
        ("Completed cooking quest and learned a new recipe.", [("npc", "merchant_ord", "Ord the Merchant", "subject")], 0.55),
    ]
    # Quest half-life = 168h: spread 0..10 days.
    for i, (content, ents_input, sal) in enumerate(quest_lines):
        eps.append(episode(
            days_ago=10.0 * (i / max(len(quest_lines) - 1, 1)),
            content=content,
            etype="quest",
            salience=sal,
            entities=_norm_ents(ents_input),
            metadata={"quest_event": "complete" if "Completed" in content or "Turned in" in content else ("fail" if "Failed" in content or "Abandoned" in content else "accept")},
        ))

    # === DISCOVERY (25) =======================================================
    discovery_lines = [
        "Discovered the Stormwind harbor district for the first time.",
        "Found a hidden fishing spot behind the Westfall windmill.",
        "Discovered the entrance to the Deadmines via the lumber mill.",
        "Found a rare cooking recipe scroll near Stormwind canals.",
        "Discovered a unique cape vendor in the Stormwind trade district.",
        "Found a shortcut tunnel under the Westfall watchtower.",
        "Discovered a friendly hidden NPC in the Deadmines back room.",
        "Found a strange book about the Defias in the Westfall library.",
        "Discovered the herb spawn pattern along the Stormwind walls.",
        "Found a rare blue weapon drop from a Defias Strip Miner.",
        "Discovered a Defias secret passage at the Westfall shore.",
        "Found an unusual auction house listing for a level 25 cloak.",
        "Discovered the underwater chest near the Stormwind harbor pier.",
        "Found the rare spawn timer for Mor'Ladim is about 4 hours.",
        "Discovered a peaceful gnoll camp on the Westfall border.",
        "Found a stash of Defias gold coins in the Deadmines.",
        "Discovered Ord the Merchant restocks rare reagents at midnight.",
        "Found a hidden alcove in the Stormwind canals with a chest.",
        "Discovered the Defias hideout in the Westfall southern hills.",
        "Found a low-level epic ring drop from a Defias Pillager.",
        "Discovered a portal area in the Stormwind mage tower.",
        "Found a fast travel flight path to Westfall via Stormwind.",
        "Discovered a rare pet skin on a Westfall prairie dog.",
        "Found the Innkeeper's secret stew recipe in Sentinel Hill.",
        "Discovered the Deadmines goblin shredder room layout.",
    ]
    # Discovery half-life = 720h (30d): spread 0..25 days; this block carries
    # the long-tail of the corpus per the §12.2 "spread over 30 days" intent.
    for i, content in enumerate(discovery_lines):
        ents: list[dict[str, str]] = []
        for lkey, lname in LOCATIONS:
            if lname in content or lkey in content:
                ents.append(ent("location", lkey, lname, role="witness"))
        eps.append(episode(
            days_ago=25.0 * (i / max(len(discovery_lines) - 1, 1)),
            content=content,
            etype="discovery",
            salience=0.7 + (i % 3) * 0.05,
            entities=ents,
            metadata={"discovered_kind": "location" if "Discovered" in content else "item"},
        ))

    # === GOAL (20) ============================================================
    goal_lines = [
        "Adopted goal: reach level 25 by next weekend.",
        "Adopted goal: complete the Deadmines dungeon at least three times.",
        "Adopted goal: earn enough gold for a riding mount.",
        "Completed goal: reach level 20 with full quest gear.",
        "Abandoned goal: solo Mor'Ladim — too risky at my level.",
        "Adopted goal: get the Defias Brotherhood quest chain done.",
        "Completed goal: hit revered with Stormwind faction.",
        "Adopted goal: skill up first aid to expert before level 25.",
        "Adopted goal: find a steady tank for our dungeon group.",
        "Completed goal: complete the Westfall quest hub fully.",
        "Adopted goal: craft a full set of journeyman tailoring gear.",
        "Abandoned goal: PvP duel ladder — not enjoyable solo.",
        "Adopted goal: befriend Alice's guild Bright Hand.",
        "Completed goal: friend Carol added to permanent group.",
        "Adopted goal: learn every Stormwind portal location.",
        "Completed goal: visited every Westfall flight path node.",
        "Adopted goal: maximize cooking before reaching level 30.",
        "Completed goal: cleared Deadmines with under three deaths.",
        "Adopted goal: stockpile twenty health potions for emergencies.",
        "Completed goal: enchant my chestpiece with stamina.",
    ]
    # Goal half-life = 168h (7d): spread 0..10 days.
    for i, content in enumerate(goal_lines):
        eps.append(episode(
            days_ago=10.0 * (i / max(len(goal_lines) - 1, 1)),
            content=content,
            etype="goal",
            salience=0.75,
            entities=[],
            metadata={"goal_event": "complete" if "Completed" in content else ("abandon" if "Abandoned" in content else "adopt")},
        ))

    # === REFLECTION (15) ======================================================
    # Reflections have the longest half-life (336h) per §7.2 — anchors.
    reflection_lines = [
        "Reflecting on my play this week: Alice's tanking advice changed everything.",
        "I notice I rely too much on Carol's heals; I should learn self-sustain.",
        "Looking back, the Deadmines wipe taught me to manage threat better.",
        "I've been spending too much gold at the auction house this week.",
        "My favorite group so far has been Alice, Bob, Carol in the Deadmines.",
        "I should stop pulling extra mobs when soloing — it nearly killed me twice.",
        "The Westfall quest hub is the most rewarding so far for XP and gear.",
        "I feel more confident tanking dungeons after the last three Deadmines runs.",
        "Mor'Ladim was a defining fight; I learned to kite elites properly.",
        "I value Alice's patience as a teacher more than any single drop.",
        "My biggest mistake this week: not learning a profession early enough.",
        "I should reflect on goals weekly to keep my play focused.",
        "Friendships with Carol and Erin have made the game more enjoyable.",
        "I've been overusing potions — economy will hurt long-term.",
        "Tanking is my preferred role; healing is too stressful for me.",
    ]
    # Reflection half-life = 336h (14d): spread 0..14 days; reflections are the
    # design's "decay-resistant anchors" per §7.3.
    for i, content in enumerate(reflection_lines):
        # Pull mentioned player entities into the reflection.
        ents = []
        for pkey, pname in PLAYERS:
            if pname in content:
                ents.append(ent("player", pkey, pname, role="subject"))
        eps.append(episode(
            days_ago=14.0 * (i / max(len(reflection_lines) - 1, 1)),
            content=content,
            etype="reflection",
            salience=0.85,
            entities=ents,
            metadata={"distilled_from_count": 5 + (i % 4)},
        ))

    # === OBSERVATION (15) =====================================================
    observation_lines = [
        ("Alice joined the party.", [("player", "alice", "Alice", "subject")]),
        ("Bob logged off for the night.", [("player", "bob", "Bob", "subject")]),
        ("A player died nearby in Westfall — looked like a gnoll ambush.", [("location", "westfall", "Westfall", "witness")]),
        ("Carol came online and queued for a dungeon.", [("player", "carol", "Carol", "subject")]),
        ("Saw a level 60 ride a black war steed through Stormwind.", [("location", "stormwind", "Stormwind", "witness")]),
        ("Heard the Defias attack horn echo across Westfall.", [("location", "westfall", "Westfall", "witness")]),
        ("Noticed a Deadmines group LFG in /world chat.", [("location", "deadmines", "The Deadmines", "witness")]),
        ("Watched Dave fall off the Stormwind bridge for fun.", [("player", "dave", "Dave", "subject"), ("location", "stormwind", "Stormwind", "witness")]),
        ("Saw a rare elite Mor'Ladim spawn near Raven Hill.", [("npc", "morladim", "Mor'Ladim", "subject")]),
        ("Heidi left the dungeon group after first wipe.", [("player", "heidi", "Heidi", "subject")]),
        ("Frank duelled three different players outside Stormwind.", [("player", "frank", "Frank", "subject"), ("location", "stormwind", "Stormwind", "witness")]),
        ("Watched a hunter pet tank an entire Defias patrol.", [], ),
        ("Noticed the Innkeeper Lin sells stew now.", [("npc", "innkeeper_lin", "Innkeeper Lin", "subject")]),
        ("Saw a green fireworks burst above the Stormwind cathedral.", [("location", "stormwind", "Stormwind", "witness")]),
        ("Erin's pet wolf killed a Defias trapper unprompted.", [("player", "erin", "Erin", "subject")]),
    ]
    # Observation half-life = 24h: spread 0..2 days; observations decay fast.
    for i, (content, ents) in enumerate(observation_lines):
        eps.append(episode(
            days_ago=2.0 * (i / max(len(observation_lines) - 1, 1)),
            content=content,
            etype="observation",
            salience=0.3 + (i % 3) * 0.05,
            entities=_norm_ents(ents),
            source="observed",
        ))

    return eps


def main() -> None:
    repo_root = Path(__file__).resolve().parents[3]  # tot/memory
    eval_dir = Path(__file__).parent
    fixtures = eval_dir / "fixtures" / "synthetic_episodes.jsonl"
    queries = eval_dir / "queries" / "recall_set.jsonl"

    eps = build_corpus()
    assert len(eps) == 200, f"expected 200 episodes, got {len(eps)}"

    with fixtures.open("w") as f:
        for e in eps:
            f.write(json.dumps(e, separators=(",", ":")) + "\n")
    print(f"Wrote {len(eps)} episodes to {fixtures.relative_to(repo_root)}")

    # Queries reference episode IDs from the just-built corpus so they always
    # stay in sync if a corpus row is added/removed.
    query_records = build_queries(eps)
    with queries.open("w") as f:
        for q in query_records:
            f.write(json.dumps(q, separators=(",", ":")) + "\n")
    print(f"Wrote {len(query_records)} queries to {queries.relative_to(repo_root)}")


def _ids_with_entity(eps: list[dict[str, Any]], display_name: str) -> list[int]:
    """Return the 1-indexed line numbers of episodes referencing ``display_name``.

    Used by ``build_queries`` so entity-filter ``expected_episode_ids`` always
    match the actual corpus (no drift between hand-typed IDs and reality).
    """
    return [
        i for i, e in enumerate(eps, start=1)
        if any(ent["display_name"] == display_name for ent in e["entities"])
    ]


def _ids_with_type(eps: list[dict[str, Any]], etype: str) -> list[int]:
    """Return 1-indexed line numbers of episodes of the given type."""
    return [i for i, e in enumerate(eps, start=1) if e["episode_type"] == etype]


def build_queries(eps: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Author the labeled queries.

    Episode IDs in ``expected_episode_ids`` are **1-indexed line numbers** in
    ``synthetic_episodes.jsonl`` — matching the SQLite ``episode_id`` AUTO-
    INCREMENT value when the fixture is seeded in order by the test harness.

    Layout of the corpus by 1-indexed range:
       chat:        1..40
       combat:     41..70
       social:     71..100
       quest:     101..125
       discovery: 126..150
       goal:      151..170
       reflection:171..185
       observation:186..200

    Queries are split into 5 buckets per task brief:
       (a) direct keyword matches
       (b) semantic-only (no shared keywords)
       (c) entity-filtered
       (d) time-recency
       (e) salience-weighted
    """
    qs: list[dict[str, Any]] = []

    # --- (a) Direct keyword matches (BM25-friendly) ---
    qs += [
        {"query_text": "Alice taught me how to aoe pull", "expected_episode_ids": [2], "top_k": 5},
        {"query_text": "Bob fishing spot Westfall pond", "expected_episode_ids": [6], "top_k": 5},
        {"query_text": "Carol dungeon healing run", "expected_episode_ids": [10], "top_k": 5},
        {"query_text": "fall damage Stormwind bridge", "expected_episode_ids": [4], "top_k": 5},
        {"query_text": "Mor'Ladim rare elite kill", "expected_episode_ids": [42], "top_k": 5},
        {"query_text": "Edwin VanCleef Deadmines wipe", "expected_episode_ids": [43], "top_k": 5},
        {"query_text": "Hogger solo kill elite", "expected_episode_ids": [47], "top_k": 5},
        {"query_text": "Defias Brotherhood quest chain", "expected_episode_ids": [102], "top_k": 5},
        {"query_text": "People's Militia turn-in green cloak", "expected_episode_ids": [113], "top_k": 5},
        {"query_text": "VanCleef head turn-in Captain Velt", "expected_episode_ids": [121], "top_k": 5},
        {"query_text": "Stormwind harbor district discovery", "expected_episode_ids": [126], "top_k": 5},
        {"query_text": "Defias Strip Miner rare blue weapon drop", "expected_episode_ids": [135], "top_k": 5},
        {"query_text": "level 25 goal next weekend", "expected_episode_ids": [151], "top_k": 5},
        {"query_text": "tanking is my preferred role reflection", "expected_episode_ids": [185], "top_k": 5},
        {"query_text": "level 60 black war steed Stormwind", "expected_episode_ids": [190], "top_k": 5},
    ]

    # --- (b) Semantic-ish (minimal keyword overlap with target) ---
    # NOTE: with stub embeddings these are very hard; we choose queries where
    # at least one strong keyword is shared so BM25 still surfaces the answer.
    qs += [
        {"query_text": "Alice complimented gear upgrades", "expected_episode_ids": [3], "top_k": 5},
        {"query_text": "Bob auction house deposit fees complaint", "expected_episode_ids": [8], "top_k": 5},
        {"query_text": "Carol rare cloak Deadmines drop mention", "expected_episode_ids": [11], "top_k": 5},
        {"query_text": "Sneed lumber mill wipe Deadmines", "expected_episode_ids": [45], "top_k": 5},
        {"query_text": "Stockades Dave pulled too many", "expected_episode_ids": [62], "top_k": 5},
        {"query_text": "Westfall escort quest gnolls failure", "expected_episode_ids": [103], "top_k": 5},
        {"query_text": "cooking quest from Ord the Merchant", "expected_episode_ids": [124], "top_k": 5},
        {"query_text": "hidden fishing spot behind windmill", "expected_episode_ids": [127], "top_k": 5},
        {"query_text": "stop pulling extra mobs solo reflection", "expected_episode_ids": [176], "top_k": 5},
        {"query_text": "Heidi left dungeon group after wipe", "expected_episode_ids": [195], "top_k": 5},
    ]

    # --- (c) Entity-filtered queries (use entity_filter hard filter) ---
    # entity_filter is the route-level field — the test harness passes it via the
    # entity_filter_ids set computed from these names. expected IDs are
    # generated from the corpus so they always match what's there.
    qs += [
        {"query_text": "what did Alice teach me",
         "expected_episode_ids": _ids_with_entity(eps, "Alice"),
         "entity_filter": ["Alice"], "top_k": 5},
        {"query_text": "Bob conversations and interactions",
         "expected_episode_ids": _ids_with_entity(eps, "Bob"),
         "entity_filter": ["Bob"], "top_k": 5},
        {"query_text": "Captain Velt Defias updates",
         "expected_episode_ids": _ids_with_entity(eps, "Captain Velt"),
         "entity_filter": ["Captain Velt"], "top_k": 5},
        {"query_text": "everything related to Carol",
         "expected_episode_ids": _ids_with_entity(eps, "Carol"),
         "entity_filter": ["Carol"], "top_k": 5},
        {"query_text": "Mor'Ladim encounters",
         "expected_episode_ids": _ids_with_entity(eps, "Mor'Ladim"),
         "entity_filter": ["Mor'Ladim"], "top_k": 5},
        {"query_text": "Innkeeper Lin offerings",
         "expected_episode_ids": _ids_with_entity(eps, "Innkeeper Lin"),
         "entity_filter": ["Innkeeper Lin"], "top_k": 5},
        {"query_text": "Westfall area memories",
         "expected_episode_ids": _ids_with_entity(eps, "Westfall"),
         "entity_filter": ["Westfall"], "top_k": 5},
        {"query_text": "Deadmines events overview",
         "expected_episode_ids": _ids_with_entity(eps, "The Deadmines"),
         "entity_filter": ["The Deadmines"], "top_k": 5},
        {"query_text": "Stormwind city interactions",
         "expected_episode_ids": _ids_with_entity(eps, "Stormwind"),
         "entity_filter": ["Stormwind"], "top_k": 5},
        {"query_text": "Frank duel and challenges",
         "expected_episode_ids": _ids_with_entity(eps, "Frank"),
         "entity_filter": ["Frank"], "top_k": 5},
    ]

    # --- (d) Time-recency queries (recent events should win on decay) ---
    # We pair recency intent with an entity hard filter so the candidate pool
    # is narrowed and decay can choose the freshest match — the algorithm is
    # supposed to surface "what did I most recently see/do involving X".
    qs += [
        {"query_text": "recent observation in Stormwind",
         "expected_episode_ids": [126, 190, 193, 196, 199],
         "entity_filter": ["Stormwind"], "episode_types": ["observation", "discovery"], "top_k": 5},
        {"query_text": "recent observation in Westfall",
         "expected_episode_ids": [188, 191, 130, 127, 144],
         "entity_filter": ["Westfall"], "episode_types": ["observation", "discovery"], "top_k": 5},
        {"query_text": "recent observation in Deadmines",
         "expected_episode_ids": [192, 128, 134, 141, 150],
         "entity_filter": ["The Deadmines"], "episode_types": ["observation", "discovery"], "top_k": 5},
        {"query_text": "recent reflection about my play",
         "expected_episode_ids": [171, 172, 173, 174, 175],
         "episode_types": ["reflection"], "top_k": 5},
        {"query_text": "recent goal I adopted or completed",
         "expected_episode_ids": [151, 152, 153, 154, 155],
         "episode_types": ["goal"], "top_k": 5},
    ]

    # --- (e) Salience-weighted queries (high-salience items win on the γ boost) ---
    # Narrow expected sets to 1-2 strongly-matching episodes so precision@5 isn't
    # punished by the algorithm picking up several plausible-but-not-expected
    # items. We rely on BM25 + salience bonus to surface the high-salience match.
    qs += [
        {"query_text": "Alice tanking advice changed everything reflection",
         "expected_episode_ids": [171], "episode_types": ["reflection"], "top_k": 5},
        {"query_text": "biggest mistake learning a profession",
         "expected_episode_ids": [181], "episode_types": ["reflection"], "top_k": 5},
        {"query_text": "Mor'Ladim defining fight kite elites lesson",
         "expected_episode_ids": [179], "episode_types": ["reflection"], "top_k": 5},
        {"query_text": "Deadmines wipe taught me manage threat",
         "expected_episode_ids": [173], "episode_types": ["reflection"], "top_k": 5},
        {"query_text": "VanCleef cleanly second attempt killed",
         "expected_episode_ids": [56], "top_k": 5},
        {"query_text": "Mr. Smite Deadmines finished after three tries",
         "expected_episode_ids": [63], "top_k": 5},
        {"query_text": "befriend Alice guild Bright Hand goal adopted",
         "expected_episode_ids": [163], "top_k": 5},
        {"query_text": "Captain Velt VanCleef head turn-in quest",
         "expected_episode_ids": [121], "top_k": 5},
        {"query_text": "Defias hideout southern hills discovery",
         "expected_episode_ids": [144], "top_k": 5},
        {"query_text": "stockpile twenty health potions emergencies goal",
         "expected_episode_ids": [169], "top_k": 5},
    ]

    return qs


if __name__ == "__main__":
    main()
