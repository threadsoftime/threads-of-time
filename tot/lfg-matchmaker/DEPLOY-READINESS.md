# LFG Matchmaker — Deploy Readiness

Status of the `lfg-matchmaker` strangler-fig slice as of the 2026-06-04 pre-deploy
hardening pass (branch `feat/lfg-matchmaker-hardening`). The slice is **built and
source-hardened but NOT yet deployed** — it ships **inert** (`LFG_ENABLED=false` by
default; mutating matchmaking actions do not run until explicitly enabled).

Specs: `azerothcore-heimdal/docs/superpowers/specs/2026-06-04-rust-lfg-matchmaker-pre-deploy-hardening-design.md`
(this pass) and `…/2026-05-29-rust-lfg-matchmaker-strangler-slice-design.md` (original
slice design — see its §10 risks).

---

## ✅ Hardened in this pass (source/test only)

| Area | What | Locking tests |
|---|---|---|
| **Reconciliation pre-form poll** (orig. spec §5/§7) | Before forming, each proposed candidate is re-confirmed live via `obs.get_state` + `roster::is_eligible` (online, ungrouped, not in combat). Any stale/errored member → proposal aborted, eligible members requeued, stale dropped. | `tick_reconciliation_aborts_form_when_a_member_went_stale`, `integration_reconciliation_aborts_when_member_stale` |
| **Invite/accept confirm-poll** (orig. spec §10) | After `bot.accept_invite`, `fulfill` bounded-polls `obs.get_group(leader)` (≤5 × 200 ms) to confirm the member joined before proceeding; on timeout → existing rollback (`bot.leave_group`). | `confirm_poll_eventually_sees_member_and_forms`, `confirm_poll_timeout_triggers_rollback`, `integration_confirm_timeout_rolls_back` |
| **Observability** | `eprintln!` → structured `tracing` across tick/orchestrator/roster + a per-tick span; **`slice-host` now inits a global `tracing-subscriber`** so the deployed (composed) service actually emits these events. Filter via `RUST_LOG`. | n/a (behavior-neutral) |
| **Blackbox integration crate** | `tests/integration.rs` drives the public API the way `slice-host` composes it (`tick::run` + `api::routes`) against a local mock harness — full cycle + the two new behaviors. | `integration_full_cycle_forms_and_places_a_balanced_5man`, `integration_routes_post_and_get_queue` (+ the two above) |

Gate: `cargo test -p lfg-matchmaker` = 80 unit + 4 integration; `cargo test --workspace`
green. No `#[ignore]`. Public API consumed by `slice-host` unchanged.

Commits: `4c182d611` (tracing) · `29cbe303f` (slice-host subscriber) · `ab1348223`
(confirm-poll) · `c883ad50d` (reconciliation) · `6cd735606` (integration crate).

---

## ⚠️ MUST verify with a LIVE smoke test before real deploy

Source/mock testing **cannot** close these — they depend on live worldserver behavior:

1. **Group instance-binding (orig. spec §10 — the single highest risk).** The slice places
   each member with `bot.enter_instance(mode="direct")` → `TeleportTo(map,x,y,z)`. Five
   independent direct teleports must land in **ONE shared instance**. If AzerothCore binds
   the instance per-entry rather than per-group at this entrance, the party **splits across
   instances**. This is the most likely spot to need a small new C++ harness primitive
   ("teleport group as a unit / bind to leader's instance"). **Do not assume the slice is
   C++-free until this is verified empirically.**
2. **Timing under real harness latency.** The new reconciliation (`obs.get_state` ×5 per
   proposal) and confirm-poll (`obs.get_group` up to ×5 per member) add round-trips. Mock
   tests run sub-ms; verify the **~2 s tick budget** still holds against real harness RTTs
   with several concurrent proposals, and that the confirm-poll's ≤1 s worst case does not
   serialize badly across a full 5-man.

### Smoke-test procedure (bots-only, deterministic, no human)
1. Deploy the slice composed in `slice-host`; set `LFG_ENABLED=true` (and `LFG_TICK_SECS`).
2. Queue N bots of assorted roles: `POST /lfg/queue {guid, role, dungeon_id, faction}`
   (slice-host mounts the slice under `/lfg`).
3. Watch the service form balanced 5-mans on its tick (the `tracing` `tick` span + `match`
   logs are now emitted via the slice-host subscriber — set `RUST_LOG=lfg_matchmaker=info`).
4. Assert via harness reads: `obs.get_group` shows a 5-member **1 tank / 1 healer / 3 dps**
   spread, and `obs.get_position` shows **all five on the same map / instance id** (this is
   the instance-binding check in risk #1).

---

## 📋 Known limitations / deferred (not blockers)

- **Retry-store not externally observable.** `PendingPlacements::{len,snapshot}` are
  `#[cfg(test)]`-only, so an external monitor (or an extended `/healthz`) cannot read the
  straggler-retry depth. Make them unconditionally public if external monitoring is needed.
- **`slice-host` startup/supervisor logging** still uses `eprintln!` by design — only the
  hosted slices were converted to `tracing`. Convert in a dedicated slice-host pass if
  uniform structured logs are wanted.
- **Production intake seam deferred** (orig. spec §6): chat `.lfg` command / addon / push
  event. Current intake is `obs.lfg_pending` drain + the `POST /lfg/queue` RPC.
- **Out of scope** (orig. spec §6): random-dungeon rewards, satchel/loot, vote-kick,
  requeue-on-disband, deserter cooldowns, cross-faction.

---

## 🚀 Deferred to the deploy session (deploy-orchestrator, kb_57b453cd)

- Add the slice/host to the release/quadlet/compose wiring (the `slice-host` image already
  composes it; confirm the quadlet sets `LFG_ENABLED` + `LFG_TICK_SECS` + `RUST_LOG`).
- Ships **inert** (`LFG_ENABLED=false`): flip to `true` only after the smoke test above
  passes, ideally as a separate enable step.
- Standard bake + mtime cross-check + soak + rollback-tag-first per kb_57b453cd.
- This hardening pass made **no live changes** — runtime images are unaffected.
