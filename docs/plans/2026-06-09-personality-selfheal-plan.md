# Plan: PersonalityCache self-heal (feat/personality-selfheal)

Date: 2026-06-09 · Spec: docs/specs/2026-06-09-personality-selfheal-design.md

## Status: IMPLEMENTED (awaiting review + merge + deploy)

## Steps

1. [x] Phase 1 (systematic-debugging): confirm root cause on-disk + live.
       - personality.rs:187-192 `unwrap_or("{}")` doesn't fire on `""`; from_str("") errs.
       - Live: bot 1083 = 140 personality_error records, all 1083, ~19.6h, ~5min cadence.
       - Brain state.sqlite living_bots[1083] has GOOD seed (Morenette/Night Elf/Warrior);
         memory store persona empty → store divergence.
       - Parity: Python original has same latent bug (not a port regression).
2. [x] TDD red: 8 failing personality tests (empty/blank/null/missing/malformed/
       empty-obj/valid-guard/no-recovery-guard) referencing PersonalityRecovery + with_recovery.
3. [x] Implement:
       - `PersonalityRecovery` trait (personality.rs).
       - `recovery: Option<Arc<dyn PersonalityRecovery>>` field + `with_recovery()` builder.
       - `get()` usable-vs-recover logic: empty/blank/null/missing/unparseable/invalid →
         recover (if wired) → persist via personality_set → cache → return; else fail-loud.
       - `recovery.rs`: `LiveRecovery` (state_store seed → obs identity → degraded, then morph)
         + shared `fill_identity_from_obs` helper + unit tests.
       - app.rs: wire LiveRecovery into PersonalityCache; refactor the two enroll obs blocks
         to the shared helper (de-dup).
4. [x] TDD green: cargo test -p brain-rs → 280 lib + 80 integration, 0 failures.
       clippy clean on recovery.rs (no new warnings).
5. [ ] Review + merge to dev (owner).
6. [ ] Deploy brain image (deploy-orchestrator). On restart, 1083 self-heals on first
       decide tick from state_store seed → morph → personality_set persists. No manual re-seed.
7. [ ] Live-verify: tail decisions.jsonl, confirm 1083 transitions off personality_error.

## 1083 recovery: SELF-HEALS ON DEPLOY (no explicit re-seed needed)

state_store seed for 1083 is intact (Morenette), so LiveRecovery layer 1 returns the
real identity with zero network calls; get() persists it. If state_store had also been
empty, layer 2 (obs.get_state) or layer 3 (degraded) would apply.
