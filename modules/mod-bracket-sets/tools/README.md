# mod-bracket-sets — extraction tools

Helpers used during Phase 1.5 step 2 (marker spell curation).

## spell_dbc_extract.py

Reads a WotLK 3.3.5a `Spell.dbc` binary and dumps tier-bonus marker
candidates (`Item - {Class} T{N} {Spec?} {2P|4P} Bonus`) where `Effect1`
is `APPLY_AURA`. Output is a tab-separated table:

```
spell_id   class         tier  spec  piece  aura_type  name
67234      Warrior       9     Melee 2      107        Item - Warrior T9 Melee 2P Bonus (Berserker Stance and Battle Stance)
...
```

### Usage

```bash
# On Heimdal, where the live Spell.dbc lives in the bind-mounted volume:
sudo cp /var/lib/containers/storage/volumes/wow-client-data/_data/dbc/Spell.dbc /tmp/Spell.dbc
sudo chmod a+r /tmp/Spell.dbc
python3 spell_dbc_extract.py /tmp/Spell.dbc > tier_bonus_candidates.tsv
```

### Field layout (verified empirically against the bundled DBC)

WotLK 3.3.5a Spell.dbc — 234 fields per record (916 bytes), 49839 records.

| Index | Name                          | Used for       |
|-------|-------------------------------|----------------|
| 0     | ID                            | spell_id       |
| 71    | Effect1                       | filter == 6 (APPLY_AURA) |
| 95    | EffectApplyAuraName1          | informational  |
| 136   | Name_lang_enUS (string offset)| name match     |

Other field positions (Effect2/3, EffectBasePoints, etc.) inferred during
probing — see `spell_dbc_probe.py` (not committed, can be regenerated).

## tier_bonus_candidates.tsv

Frozen snapshot of all 115 tier-bonus markers in our bundled Spell.dbc
(captured 2026-05-13). Used by step 2 of the Phase 1.5 implementation to
pick 54 distinct marker IDs for `bracket_set_bonus_map`.

To regenerate: re-run `spell_dbc_extract.py` after a DBC update.

## Cross-check methodology (manual step)

After picking candidate IDs, grep AzerothCore's spell scripts to find any
that already bind one of your picks. Avoid those (their existing
SpellScript would conflict with ours):

```bash
ssh heimdal "grep -rE '\\b(<comma-separated IDs>)\\b' \
    /opt/containers/wow/source/src/server/scripts/Spells/"
```

Conflicted IDs from the 2026-05-13 pass (do not use as markers):
64736, 64869, 64890, 64895, 64928, 67151, 67191, 67201, 67228, 70650,
70656, 70718, 70723, 70726, 70752, 70770, 70803, 70805, 70811, 70817,
70832, 70847.
