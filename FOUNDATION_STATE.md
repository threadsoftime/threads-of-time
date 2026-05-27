# Threads of Time — Foundation Complete

This file is a one-time record of the state at the `foundation-complete` tag (2026-05-27).

## What works

- Worldserver builds cleanly from `tot/release/build.sh` (37 min on Heimdal with `-j4`, ccache cold)
- All migrated modules compile and link: mod-bracket-sets, mod-rotation-mode, mod-warforged, mod-harness-bridge, mod-agenticbots (scaffold-only)
- mod-playerbots populated into the build tree from `/opt/containers/wow/source/modules/mod-playerbots/` (Heimdal's existing install); falls back to git clone of upstream `liyunfan1223/mod-playerbots` if absent
- Brain sidecar at `tot/brain/` runs in single-endpoint mode (BYOLLM)
- V3.8 multi-mode router placeholder at `tot/internal-docs/heimdal-router/` (no actual code; was spec-only)
- First worldserver image baked: `localhost/wow-server:tot-20260527` (712MB)
- DB-import image: `localhost/wow-db-import:tot-20260527` (1.27GB)

## What's stubbed for Plans 2-6

- **Memory subsystem** (`tot/memory/`): scaffold README only. Plan 2 implements the full V3 schema + sqlite-vec + hybrid retrieval + `memory.*` tools.
- **Client MPQ composition** (`tot/client-patch/`): dbc-patch-builder + compose-tot-addon migrated as-is. Plan 4 generalizes the MPQ pack stage with DBC ID-range collision detection.
- **Reference deploy stack** (`tot/deploy/`): Heimdal-specific Quadlets carried verbatim. Plan 5 generalizes into operator-portable `.env`-driven stack + install script + first-boot bootstrap.
- **Release pipeline** (`tot/release/`): only `build.sh` migrated. Plan 6 adds `release.sh`, GHCR push, GitHub Release creation, nightly AC drift CI.
- **BFD-tuned bot strategies**: 8 overlay files committed under `modules/mod-agenticbots/pending-upstream-pr/` (outside the AC source glob). They activate in 1.1.0+ when the mod-playerbots upstream PR adding `RegisterCustomStrategyContext`/etc. lands; see `tot/internal-docs/agenticbots-upstream-prs.md`.

## What's pinned

- AC SHA: `6d83e35d19f65a0124b7f6e9588289db05eda0df` (see `UPSTREAMS.toml [ac]`)
- mod-playerbots tested SHA: `a02e4ca5c17e636ac0e36277108480293ab8358c` (see `UPSTREAMS.toml [playerbots-dependency]`)
- License: GPL-2.0-or-later (matches AC literally at the pinned SHA)
- Foundation tag: `foundation-complete`

## Known issues + follow-up work

1. **`tot/release/build.sh` line 159** has `env/dist/etc/modules` where it should be `env/ref/etc/modules` (conf-staging path). Conf files were staged to `tot-staged/` directory instead of being merged into the live conf tree. Non-blocking; noted in `kb_87a7eade` item 23 for the next `build.sh` fix pass.
2. **Brain env var name mismatch**: existing brain code uses `LLM_BASE_URL`; spec uses `BRAIN_LLM_URL`. Reconciliation deferred to Plan 2 (memory subsystem ships the BYOLLM contract).
3. **mod-agenticbots BFD strategies** are unwired pending mod-playerbots upstream PR; bots in BFD raid play with generic strategies in 1.0.0.
4. **3 of Task 5's HOOK extractions** carry whitespace-only reformatting noise in `Guild.cpp/h` + `CharacterHandler.cpp`. Future cleanup deferred to a "squash-vs-stock-AC rebase" pass.
5. **Source-build operators** need to clone mod-playerbots into `modules/mod-playerbots/` before running the build. Pre-built container users don't see this. Spec §3.6 + §6.1 updated to make this explicit.

## Next plan

Plan 2 (V3 memory subsystem) is the longest item on the critical path for 1.0.0 and should start immediately. See `docs/superpowers/plans/2026-05-27-threads-of-time-1.0.0-memory-subsystem.md`.

Plans 3-6 can proceed in parallel after Plan 2's Phase 1 design pass lands.
