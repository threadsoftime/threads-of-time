# Threads of Time — DBC ID-range allocation

Each module shipping client DBC rows declares its owned ID ranges in
`modules/<mod>/client/MANIFEST.toml` under `[id_ranges]`. The Stage B
compositor (`pack-mpq.py`) fails loud if two modules claim overlapping ranges
for the SAME DBC file (check A), and if any recipe emits a row outside its
declared ranges (check B / §10.3 stock-row gate). This file is the
human-readable map; the manifests are the source of truth. Update this table
in the same commit as any range change.

| DBC file                  | Ranges                                   | Owner            | Notes |
|---------------------------|------------------------------------------|------------------|-------|
| ItemSet.dbc               | 90100–90199                              | mod-bracket-sets | 90101 fallback + 90111..90193 (27 class+spec) |
| Spell.dbc                 | 64854–64939, 67121–67268, 70724–70841    | mod-bracket-sets | 54 marker name/description overrides (3 clusters) |
| SpellItemEnchantment.dbc  | 70001–70063                              | mod-warforged    | 21 enchant rows; CLIENT ships partial (ToT rows only, §10.3); server builds the full file separately |

Collision is per-file: warforged's 70001–70063 does NOT collide with
bracket-sets' Spell.dbc cluster 70724–70841 because they are different files.

## Adding a new DBC contributor

1. Pick unused ranges for each DBC file you touch (check this table).
2. Add `[id_ranges]` to your `modules/<mod>/client/MANIFEST.toml`.
3. Add a row to the table above.
4. `python tot/client-patch/pack-mpq.py --version 0.0.0-dev` — a clean run proves
   no overlap and no stray rows; a violation aborts with a message naming the module.
