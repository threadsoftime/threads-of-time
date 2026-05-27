# Threads of Time

An unofficial, non-commercial fan-built expansion for *World of Warcraft: Wrath of the Lich King 3.3.5a*, built on [AzerothCore](https://github.com/azerothcore/azerothcore-wotlk).

**Threads of Time is not affiliated with, endorsed by, or sponsored by Blizzard Entertainment, Inc. World of Warcraft® and Wrath of the Lich King® are trademarks of Blizzard Entertainment. All Blizzard intellectual property remains the property of Blizzard Entertainment.**

## What this is

Threads of Time (ToT) is an AI-driven WoW expansion. The headline feature is **alive bots** — LLM-orchestrated characters with persistent memories, personalities, and goal-directed behavior — playing alongside you through custom bracketed content.

ToT 1.0.0 covers Bracket 1 (levels 1–25), with class+spec-specific tier-set bonuses, a bracket-appropriate raid (Blackfathom Deeps), Legion-style Warforged + Bonus Socket procs on item drops, and the rotation mode system.

## Install

See [docs/install.md](docs/install.md) for the operator install path (full instructions ship in Plan 5).

For players: download `patch-Z-tot-1.0.0.MPQ` from the releases page, drop into your client's `Data/` folder, edit `Data/<locale>/realmlist.wtf` to point at your server's realmlist, and log in.

## Architecture

See [`docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md`](docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md) for the full release-architecture design.

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE) for the full text. See [CONTRIBUTING.md](CONTRIBUTING.md) for the contributor license.

## Dependencies

ToT depends on [mod-playerbots](https://github.com/liyunfan1223/mod-playerbots) being installed alongside (see [UPSTREAMS.toml](UPSTREAMS.toml) for the supported version range). The install script (Plan 5) verifies this for you.
