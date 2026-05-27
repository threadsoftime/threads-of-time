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

### Known upstream PR

The BFD instance-strategy registration (Plan 1 Task 18) requires 4 public static methods (`RegisterCustomStrategyContext`, `RegisterCustomActionContext`, `RegisterCustomTriggerContext`, `RegisterCustomInstanceStrategy`) to be added to mod-playerbots. See `tot/internal-docs/agenticbots-upstream-prs.md` for the PR tracker (created in Task 18).

## License

GPL-2.0-or-later (matches AC).

## Architecture

[Architecture overview — populated as features land. Currently scaffolded with the WorldScript entry point + initial BFD strategy overlays lifted from patches/mod-playerbots-bfd/.]
