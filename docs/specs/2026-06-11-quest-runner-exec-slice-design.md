# QuestRunner Exec Slice — Design (M3 Gate 3)

**Date:** 2026-06-11  
**Status:** IMPLEMENTED (v1)  
**Lane:** Rust exec-rs + tot-goal-contract  
**Cross-refs:** kb_9978dfe3 (M3 pillar), kb_87a7eade (state), recon9 rider specs  
**Prerequisites:** Phase 1 (harness-rs registry) + C++ adapters in worldserver bce7a35ee193

---

## 1. Scope

A `Quest` goal variant in `tot-goal-contract` and a `run_quest` executor in
`exec-rs` that sequences one quest from accept through objective completion to
turnin. Consumes the 5 M3 harness verbs (`bot.accept_quest`, `bot.turnin_quest`,
`bot.use_item`, `bot.interact_object`, existing `scan/fight/loot`) and the
existing movement primitives (`walk_far`).

**Out of scope v1:** escort quests, vehicle quests, transport steps (boat/zeppelin),
targeted `bot.use_item` (phase-2), multi-quest chaining (that is the brain's job).

---

## 2. QuestGoal contract

```
QuestGoal {
  quest_id:      u32,
  title:         String,          // display only
  giver:         QuestGiver,      // accept step target
  receiver:      QuestGiver,      // turnin step target (may be different NPC)
  steps:         Vec<QuestStep>,  // ordered objective steps (between accept & turnin)
  step_timeout_s: u64,            // per-step time budget (default 600)
  total_timeout_s: u64,           // whole-quest time budget (default 3600)
}

QuestGiver {
  kind:       GiverKind,    // Npc | GameObject
  entry:      u32,          // creature_template or gameobject_template entry
  pos:        WorldPos,     // approach position (walk_far destination)
  // quest_giver_guid is resolved at runtime via obs.get_nearby_hostiles /
  // obs.query_db; NOT stored in the profile (would be stale on server restart).
}

enum GiverKind { Npc, GameObject }

enum QuestStep {
  // Navigate to a position (walk_far).
  GoTo { pos: WorldPos, label: Option<String> },
  
  // Kill N mobs of a given entry to satisfy a kill-credit objective.
  // Completion predicate: obs.get_quest_log objective[index].current >= required.
  Kill { mob_entry: u32, count: u32, site: WorldPos, obj_index: u8,
         search_radius: f32, mob_level_min: u32, mob_level_max: u32 },
  
  // Collect N of an item by killing mobs that drop it.
  // Completion predicate: obs.get_quest_log objective[index].current >= required.
  Collect { item_entry: u32, count: u32, mob_entry: u32, site: WorldPos,
             obj_index: u8, search_radius: f32,
             mob_level_min: u32, mob_level_max: u32 },
  
  // Use a specific item (no target — v1 only; targeted path = phase-2).
  // Completion predicate: obs.get_quest_log objective[index].current >= required.
  UseItem { item_entry: u32, obj_index: u8 },
  
  // Interact with a gameobject (preferred: spawn_guid from runtime obs.query_db;
  // fallback: entry + search_range).
  // Completion predicate: obs.get_quest_log objective[index].current >= required.
  InteractObject { object_entry: u32, site: WorldPos, search_range: f32,
                   obj_index: u8, count: u32 },
}
```

The QuestRunner step sequence is: accept → steps[0..N] → turnin.

---

## 3. Completion predicates

Every step evaluates against `obs.get_quest_log {target_guid: bot_guid}`.

| Step | Completion condition |
|------|----------------------|
| Kill / Collect / InteractObject | `objectives[obj_index].current >= objectives[obj_index].required` |
| UseItem | `objectives[obj_index].current >= objectives[obj_index].required` |
| GoTo | arrival: `distance(bot_pos, dest) <= ARRIVAL_YARDS` (from nav.rs) |

Quest overall-complete: `status == "complete"` in the quest log entry.

---

## 4. State machine

```
QuestState {
  Approaching { stage: ApproachStage },   // walking to giver/receiver/site
  Accepting,
  StepRunning { step_idx: usize },
  TurningIn,
  Done,       // terminal Completed
  Failed { reason },  // terminal NeedsDecision or Blocked
}

enum ApproachStage { Giver, StepSite { idx: usize }, Receiver }
```

Each state transition calls one harness verb or `walk_far`, then re-evaluates the
completion predicate. Per-step and total timeouts emit `NeedsDecision` rather than
spin.

---

## 5. EmissionLedger integration

`run_quest` returns `GoalStatus`. Timeouts → `GoalStatus::NeedsDecision { event:
EscalationEvent::PathStuck }` (for navigation failures) or `GoalStatus::Blocked`
(for step-timeout exhaustion). The EmissionLedger Blocked=300s / NeedsDecision=600s
cooldowns apply via the existing brain-side machinery unchanged.

---

## 6. Giver GUID resolution (v1 approach)

The `QuestGiver.pos` anchors a `walk_far` approach. After arriving, the exec
calls `obs.get_nearby_hostiles` (for NPCs) or a future `obs.get_nearby_objects`
to resolve the live packed uint64 GUID. For v1: use `obs.query_db` with a
template to resolve the spawn's live GUID from the DB prior to the call.

**Open question OQ-1:** Should we add an `obs.get_nearby_objects` tool (equivalent
to `obs.get_nearby_hostiles` for gameobjects)? For v1, the entry-search mode of
`bot.interact_object` (object_entry + search_range) sidesteps this for GOs. For
NPCs, `obs.get_nearby_hostiles` gives us GUIDs. Conservative choice: use
entry-search for GOs in v1 accept/turnin, add nearby-objects later.

**Open question OQ-2:** The giver-guid resolution path for `bot.accept_quest` and
`bot.turnin_quest` requires a packed uint64. For v1: call `obs.get_nearby_hostiles`
and match on creature entry. If no match within range, return
`GoalStatus::Blocked { reason: Other, detail: "giver_not_in_range" }`.

---

## 7. v1 limitations (skip-tagged)

- Escort quests (`tags: ["escort"]`): emit `GoalStatus::Blocked { reason: Other, detail: "escort_skip_v1" }` immediately.
- Vehicle quests (`tags: ["vehicle"]`): same.
- `UseItem` with `target_guid != 0`: return `fail_code: "targeted_phase2"` from the C++ adapter; exec logs and continues (if the objective has another path) or emits NeedsDecision.
- Multi-step interact (count > 1 for InteractObject): loop the interact call N times, moving to the site between each.

---

## 8. Design choices (documented for future review)

| Decision | Choice | Rationale |
|----------|---------|-----------|
| Step order determinism | Strictly sequential | Profiles are ordered; parallel objectives would require multi-bot coordination |
| Objective progress source | `obs.get_quest_log` polled after each harness call | Single source of truth; avoids client-side counters that can drift |
| Per-step timeout | 600s default (configurable per goal) | LK quest step budgets: 20-min steps cover the worst Howling Fjord grind |
| Total timeout | 3600s | One questing session; brain decides whether to re-emit |
| Kill/Collect step execution | Delegates to `run_grind` logic (reuses scan/fight/loot machinery) | Avoids duplicating the whole grind loop; quest-complete predicate is the stop condition |
| Giver GUID resolution | Walk to giver pos, then obs.get_nearby_hostiles | No out-of-range DB probe needed; consistent with M2 combat approach |

