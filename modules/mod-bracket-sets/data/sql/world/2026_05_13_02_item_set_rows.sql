--
-- mod-bracket-sets — itemset_dbc row for Bracket 1
--
-- This playerbots AzerothCore fork stores set metadata in itemset_dbc
-- (the DBC-format SQL mirror), not stock AC's `item_set` table — that
-- table doesn't exist here. The mod bypasses the engine's set-effect
-- path entirely (via OnPlayerCanApplyEquipSpellsItemSet) and applies
-- its own bonus auras based on bracket_set_bonus_map, so SetSpellID_*
-- and SetThreshold_* stay 0.
--
-- Name_Lang_enUS drives the in-game inventory UI label
-- "Vestiges of Blackfathom (N/M)" when wearing items with itemset=90101.
--
-- ItemID_1..17 are left 0 because the bracket-1 set has 30+ items
-- (more than DBC's 17-slot capacity). Engine determines set membership
-- from item_template.itemset (populated by pack 03), not these slots.
--

REPLACE INTO `itemset_dbc`
  (`ID`,
   `Name_Lang_enUS`,
   `ItemID_1`, `ItemID_2`, `ItemID_3`, `ItemID_4`, `ItemID_5`, `ItemID_6`, `ItemID_7`, `ItemID_8`, `ItemID_9`,
   `ItemID_10`, `ItemID_11`, `ItemID_12`, `ItemID_13`, `ItemID_14`, `ItemID_15`, `ItemID_16`, `ItemID_17`,
   `SetSpellID_1`, `SetSpellID_2`, `SetSpellID_3`, `SetSpellID_4`, `SetSpellID_5`, `SetSpellID_6`, `SetSpellID_7`, `SetSpellID_8`,
   `SetThreshold_1`, `SetThreshold_2`, `SetThreshold_3`, `SetThreshold_4`, `SetThreshold_5`, `SetThreshold_6`, `SetThreshold_7`, `SetThreshold_8`,
   `RequiredSkill`, `RequiredSkillRank`)
VALUES
  (90101,
   'Vestiges of Blackfathom',
   0, 0, 0, 0, 0, 0, 0, 0, 0,
   0, 0, 0, 0, 0, 0, 0, 0,
   0, 0, 0, 0, 0, 0, 0, 0,
   0, 0, 0, 0, 0, 0, 0, 0,
   0, 0);
