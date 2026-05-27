# pending-upstream-pr/

BFD-tuned bot strategies for mod-agenticbots. Lifted from `patches/mod-playerbots-bfd/` in Task 16, but **NOT YET COMPILED INTO THE WORLDSERVER** because they depend on mod-playerbots's `Action.h`, `AttackAction.h`, etc. headers — which aren't in the ToT repo (mod-playerbots is an external runtime dependency operators install at deploy time, not a build-time vendored upstream).

## Why this directory exists outside `src/`

AzerothCore's CMake module system globs `src/**/*.cpp` for each module. Anything under `src/` is compiled. Files here are deliberately outside `src/` so the glob skips them — they're committed for future activation (1.1.0+) but inert for 1.0.0 builds.

## When this activates

Once the upstream PR adding 4 public custom-context registration methods to mod-playerbots is accepted (see `tot/internal-docs/agenticbots-upstream-prs.md` PR-1):

1. Operators install the new mod-playerbots version (`UPSTREAMS.toml` `[playerbots-dependency].min` bumps to the accepting version)
2. mod-agenticbots's build picks up mod-playerbots's headers from the sibling `modules/mod-playerbots/` install (via AC's standard inter-module include path mechanism)
3. These files move back into `src/{action,strategy,trigger}/bfd/` and become reachable
4. `src/glue/AgenticbotsRegistration.cpp` (new in 1.1.0) registers the BFD strategies via the upstream PR's new methods

## 1.0.0 user-visible impact

Bots in BFD raid play with generic mod-playerbots strategies (not BFD-tuned). All other "alive bot" features (memory, decision loop, subset gating, harness integration) work normally.

Refs: spec §1.2, §9.2; Task 14 inventory; Task 18 PR tracker.
