# Threads of Time AddOn — priority allocation

The unified `ThreadsOfTime` AddOn (composed by `compose-tot-addon.py` from per-mod
`modules/<mod>/data/addon-contrib/` directories) loads files in `(priority, mod, file-index)` order.

Priority bands (lower loads earlier):

| Band | Reserved for |
|---|---|
| 0–9   | Core/shared namespace, utilities, the auto-generated `00-Core.lua` |
| 10–89 | Feature mods (mod-warforged, mod-bracket-sets tier-set UI, future mods) |
| 90–99 | Integration / glue / overrides that need to run last |

Current allocations:

| Mod | Priority | Notes |
|---|---|---|
| mod-warforged | 10 | WarforgedStatBumps + GameTooltip + ItemRef + WarforgedSentinel |
| mod-bracket-sets | reserved 11 | tier-set UI Lua (when authored; currently `data/addon-contrib/.gitkeep`) |

When adding a new contributing mod, pick the lowest unused priority in the appropriate band and update this table in the same commit as the manifest.toml.
