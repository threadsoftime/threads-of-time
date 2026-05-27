# mod-agenticbots

ToT's bot-AI layer. Adds LLM-orchestrated strategies, actions, triggers, harness integration, and the brain wiring on top of mod-playerbots.

## Dependency: mod-playerbots

mod-agenticbots **depends on** [mod-playerbots](https://github.com/liyunfan1223/mod-playerbots) being installed alongside in the AC `modules/` directory. mod-agenticbots calls into mod-playerbots's public API (`PlayerbotAI`, `AI_VALUE`/`AI_VALUE2` macros, the strategy/action/trigger system, `GET_PLAYERBOT_AI(player)`).

The supported mod-playerbots version range is declared in the top-level `UPSTREAMS.toml`:

```toml
[playerbots-dependency]
min    = "..."   # minimum tested version
tested = "..."   # exact version CI runs against
```

mod-agenticbots does **not** vendor, modify, or redistribute mod-playerbots's source. If a mod-agenticbots feature requires changes to mod-playerbots internals, that's either (a) a request to upstream, (b) a documented hard requirement on a specific mod-playerbots version, or (c) a redesign to use the public API.

### Known upstream PR + 1.0.0 scope deferral

The BFD instance-strategy registration requires 4 public static methods (`RegisterCustomStrategyContext`, `RegisterCustomActionContext`, `RegisterCustomTriggerContext`, `RegisterCustomInstanceStrategy`) to be added to mod-playerbots. See [`tot/internal-docs/agenticbots-upstream-prs.md`](../../tot/internal-docs/agenticbots-upstream-prs.md) for the PR tracker.

**ToT 1.0.0 scope deferral:** Until that upstream PR lands and `[playerbots-dependency].min` in `UPSTREAMS.toml` is bumped, mod-agenticbots ships in a degraded mode:

- The 8 BFD-tuned strategy files in `src/{strategy,action,trigger}/bfd/` are compiled into the worldserver but not registered with mod-playerbots's strategy factory (no glue in `Addmod_agenticbotsScripts()`).
- Bots in BFD raid play with generic mod-playerbots strategies (not BFD-tuned).
- All other "alive bot" features (memory subsystem, decision loop, subset gating, harness integration) work normally — only the BFD-specific tactical layer is deferred.

Activates in 1.1.0+ once the upstream PR is accepted.

## License

GPL-2.0-or-later (matches AC).

## Architecture

[Architecture overview — populated as features land. Currently scaffolded with the WorldScript entry point + initial BFD strategy overlays lifted from patches/mod-playerbots-bfd/.]
