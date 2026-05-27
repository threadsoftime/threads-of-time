# mod-bracket-sets

AzerothCore module for Heimdal that implements **bracket-locked tier set bonuses with spec-morph dispatch**.

Behavior in one sentence: when a player equips 2 or 4 pieces of a Heimdal bracket itemset (90101-90199), this module looks up the (itemset, threshold, class, spec) tuple in `bracket_set_bonus_map` and applies the matching aura — but only while the player's level is inside the bracket's window (e.g. 25-34 for Bracket 1).

## Status

**v0 — scaffolding only.** Module registers a no-op PlayerScript and loads its config. No bonuses fire. The framework is here so subsequent PRs can fill in Registry → Manager → BonusEffects → SQL seed without touching the build pipeline again.

Tracked work: see [Phase 1.5 implementation plan](../../docs/superpowers/plans/2026-05-13-phase-1-5-bracket-1-tier-sets-implementation.md).

## Files

| File                                    | Role                                                                  |
|-----------------------------------------|-----------------------------------------------------------------------|
| `conf/mod_bracket_sets.conf.dist`       | Single `Enable` flag. Default `1`.                                    |
| `src/bracket_sets_loader.cpp`           | Entry point `Addmod_bracket_setsScripts()` → calls registration funcs |
| `src/BracketSets.h`                     | Umbrella header.                                                      |
| `src/BracketSetsConfig.{h,cpp}`         | Config loader (reads `BracketSets.Enable`).                           |
| `src/BracketSetsRegistry.{h,cpp}`       | Caches `bracket_set_bonus_map` rows in memory at boot.                |
| `src/BracketSetsManager.{h,cpp}`        | Per-player apply/remove logic for tracked bonus auras.                |
| `src/BracketSetsPlayerScript.{h,cpp}`   | Hooks the 5 `PlayerScript` events: equip / login / level / spec / can-apply. |
| `src/BracketSetsBonusEffects.{h,cpp}`   | The 54 AuraScript/SpellScript classes implementing each bonus.        |

## Integration

`image/build.sh` rsyncs this entire `modules/mod-bracket-sets/` tree into the worldserver build clone after URL-based modules are checked out. AC's auto-discovery picks it up; no manual entry in `modules.txt` needed.

## License

Same as AzerothCore: GNU AGPL v3.
