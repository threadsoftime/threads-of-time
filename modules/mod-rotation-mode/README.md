# mod-rotation-mode

AzerothCore module for Heimdal implementing the **PE2 rotation-mode wrapper** over upstream mod-playerbots' `.playerbots bot self` toggle. Adds:

- A `.heimbot` chat-command surface (mode toggle, per-character settings get/set, list, reset).
- A per-character `heimbot_settings` KV store in `acore_characters`.
- Class+spec → ON/OFF strategy-set composition for "rotation mode" (combat-only, no movement, no target change).
- An optional `HealTargetSourceStrategy` override (deferred; design in PE2 brainstorm).

Behavior in one sentence: when the player types `.heimbot mode rotation`, the module engages `.playerbots bot self` and applies a strategy set that lets the AI run combat rotation + healing while the player keeps movement, target, and camera control.

## Status

**v0 — scaffolding only.** Module registers the command surface and a no-op PlayerScript. Commands log to the player's chat but don't yet alter bot strategies. The DB table is created at first boot.

Tracked work: see [Player Experience PE1-PE3 brainstorm](../../docs/superpowers/specs/2026-05-13-player-experience-pe1-pe3-brainstorm.md). PE1 (config flip) is deployed (`kb_652218dc`); PE2 implementation continues from this scaffold.

## Files

| File                                              | Role                                                          |
|---------------------------------------------------|---------------------------------------------------------------|
| `conf/mod_rotation_mode.conf.dist`                | Single `Enable` flag. Default `1`.                            |
| `data/sql/characters/...heimbot_settings_table.sql` | DDL for the per-character KV table. Auto-applied at boot.    |
| `src/rotation_mode_loader.cpp`                    | `Addmod_rotation_modeScripts()` entry point.                  |
| `src/RotationMode.{h,cpp}`                        | Umbrella + WorldScript (config load, table create check).     |
| `src/RotationModeConfig.{h,cpp}`                  | Config loader.                                                |
| `src/HeimbotSettings.{h,cpp}`                     | Per-character KV store reads/writes.                          |
| `src/HeimbotCommandScript.{h,cpp}`                | `.heimbot` chat command surface (mode, set, get, list, reset).|
| `src/RotationModePlayerScript.{h,cpp}`            | OnLogin hook: re-apply saved mode for the character.          |
| `src/StrategySetComposer.{h,cpp}`                 | Class+spec → ON/OFF strategy lists for each mode.             |

## Modes

- `off` — release any AI control; vanilla behavior.
- `rotation` — AI runs combat + heal only; player owns movement + target. **Primary PE2 feature.**
- `grind` — full autopilot for AFK leveling. Future work.
- `squad-mirror` — player drives; bot squad mirrors via existing "reactions" behaviors. Future work.

## Settings (PE2 v1 surface)

Per-character keys (see PE2 brainstorm for full list):

- `mode` — global mode (off / rotation / grind / squad-mirror)
- `safety.pause-on-input-seconds`, `safety.combat-only`
- `rotation.flash-heal-threshold`, `rotation.panic-heal-threshold`, etc.
- `rotation.heal-target-source` (lowest-friendly / master-selection / tank-priority)
- `rotation.dot-refresh-window`, `rotation.movement-cast-policy`
- `rotation.cooldowns.<name>` (always / boss-only / manual)
- `squad.disperse-yards`, `squad.rti-cc-icon`, `squad.rti-kill-icon`, `squad.auto-revive`
- `grind.loot-threshold`, `grind.gather.*`, `grind.bag-full-action`, `grind.accept-quests`

## License

GNU AGPL v3 (matches AzerothCore).
