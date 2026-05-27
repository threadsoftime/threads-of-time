--
-- Phase 1.1 Step 8 — BFD raid-tier loot items
--
-- 14 new items in entries 90200-90216. Slot/stat profile is roughly
-- 10-15% above the existing dungeon-tier Bracket 1 items, all
-- RequiredLevel=25, Quality=4 (Epic).
--
-- Seven tier pieces (itemset=90101) extend the existing "Tidesworn"
-- progression -- equipping a raid tier piece counts toward the same
-- 2pc/4pc bonuses as the dungeon-tier pieces, so a partial raid
-- clear plus existing dungeon gear still triggers the framework.
--
-- Seven non-tier items are flavor drops (weapons / jewelry / relic).
-- They don't carry the itemset binding -- pure stat upgrades.
--
-- Each item also gets a bracket_personal_loot row at 2% chance per
-- player per kill. Lower than the 4% dungeon-tier drops because
-- raid pieces should feel rare.
--
-- Stat type cheat sheet (item_template.stat_type*):
--   3 = AGILITY    4 = STRENGTH    5 = INTELLECT
--   6 = SPIRIT     7 = STAMINA    12 = DEFENSE_SKILL_RATING
--   31 = HIT_RATING  32 = CRIT_RATING  35 = RESILIENCE
--   45 = SPELL_POWER  44 = ARMOR_PENETRATION  48 = BLOCK_RATING
--

DELETE FROM `item_template` WHERE `entry` BETWEEN 90200 AND 90216;

-- ============================================================================
-- TIER PIECES (itemset = 90101)
-- ============================================================================

-- 90200 Tidesworn Pauldrons (Plate shoulders, Ghamoo-ra drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `armor`, `bonding`, `itemset`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`,
    `stat_type3`, `stat_value3`, `description`)
VALUES (90200, 4, 4, 'Tidesworn Pauldrons', 5027,
    4, 0, 3, -1,
    34, 25, 460, 1, 90101,
    4, 18, 7, 14,
    12, 8, 'Bracket 1 Tier (raid) -- counts toward 2pc/4pc bonuses.');

-- 90201 Tidesworn Robes (Cloth chest, Lady Sarevess drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `armor`, `bonding`, `itemset`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`,
    `stat_type3`, `stat_value3`, `description`)
VALUES (90201, 4, 1, 'Tidesworn Robes', 1820,
    4, 0, 5, -1,
    34, 25, 85, 1, 90101,
    5, 22, 7, 16,
    6, 14, 'Bracket 1 Tier (raid) -- counts toward 2pc/4pc bonuses.');

-- 90202 Tidesworn Hauberk (Mail chest, Old Serra'kis drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `armor`, `bonding`, `itemset`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`,
    `stat_type3`, `stat_value3`, `description`)
VALUES (90202, 4, 3, 'Tidesworn Hauberk', 3070,
    4, 0, 5, -1,
    34, 25, 380, 1, 90101,
    3, 20, 7, 16,
    5, 12, 'Bracket 1 Tier (raid) -- counts toward 2pc/4pc bonuses.');

-- 90203 Tidesworn Leggings (Leather legs, Gelihast drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `armor`, `bonding`, `itemset`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`,
    `stat_type3`, `stat_value3`, `description`)
VALUES (90203, 4, 2, 'Tidesworn Leggings', 12001,
    4, 0, 7, -1,
    34, 25, 230, 1, 90101,
    3, 22, 7, 18,
    4, 8, 'Bracket 1 Tier (raid) -- counts toward 2pc/4pc bonuses.');

-- 90204 Tidesworn Helm (Plate head, Lorgus Jett drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `armor`, `bonding`, `itemset`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`,
    `stat_type3`, `stat_value3`, `description`)
VALUES (90204, 4, 4, 'Tidesworn Helm', 21029,
    4, 0, 1, -1,
    34, 25, 490, 1, 90101,
    4, 17, 7, 14,
    12, 10, 'Bracket 1 Tier (raid) -- counts toward 2pc/4pc bonuses.');

-- 90205 Tidesworn Headcover (Cloth head, Kelris drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `armor`, `bonding`, `itemset`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`,
    `stat_type3`, `stat_value3`, `description`)
VALUES (90205, 4, 1, 'Tidesworn Headcover', 6411,
    4, 0, 1, -1,
    34, 25, 70, 1, 90101,
    5, 20, 7, 14,
    6, 12, 'Bracket 1 Tier (raid) -- counts toward 2pc/4pc bonuses.');

-- 90206 Tidesworn Chestguard (Plate chest, Aku'mai drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `armor`, `bonding`, `itemset`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`,
    `stat_type3`, `stat_value3`, `description`)
VALUES (90206, 4, 4, 'Tidesworn Chestguard', 21028,
    4, 0, 5, -1,
    35, 25, 610, 1, 90101,
    4, 22, 7, 20,
    12, 10, 'Bracket 1 Tier (raid) -- counts toward 2pc/4pc bonuses.');

-- ============================================================================
-- NON-TIER ITEMS (no itemset)
-- ============================================================================

-- 90210 Adamant Maul (2H mace, Ghamoo-ra drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `bonding`,
    `dmg_min1`, `dmg_max1`, `dmg_type1`, `delay`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`)
VALUES (90210, 2, 5, 'Adamant Maul', 1542,
    4, 0, 17, -1,
    35, 25, 1,
    80, 130, 0, 3500,
    4, 18, 7, 15);

-- 90211 Naga Sea-Witch's Wand (wand, Lady Sarevess drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `bonding`,
    `dmg_min1`, `dmg_max1`, `dmg_type1`, `delay`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`)
VALUES (90211, 2, 19, "Naga Sea-Witch's Wand", 5277,
    4, 0, 26, -1,
    35, 25, 1,
    35, 65, 6, 1700,
    5, 12, 6, 10);

-- 90212 Razor-Fin Tooth Necklace (Old Serra'kis drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `bonding`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`,
    `stat_type3`, `stat_value3`)
VALUES (90212, 4, 0, 'Razor-Fin Tooth Necklace', 6494,
    4, 0, 2, -1,
    35, 25, 1,
    3, 12, 7, 10,
    32, 15);

-- 90213 Murloc Slayer's Mitts (Cloth gloves, Gelihast drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `armor`, `bonding`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`,
    `stat_type3`, `stat_value3`)
VALUES (90213, 4, 1, "Murloc Slayer's Mitts", 1844,
    4, 0, 10, -1,
    34, 25, 55, 1,
    5, 14, 7, 12,
    6, 10);

-- 90214 Twilight Cult Libram (Lorgus Jett drop, paladin relic)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `bonding`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`)
VALUES (90214, 4, 7, 'Twilight Cult Libram', 23472,
    4, 0, 28, 2,
    35, 25, 1,
    45, 28, 5, 12);

-- 90215 Mind Render's Dagger (Kelris drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `bonding`,
    `dmg_min1`, `dmg_max1`, `dmg_type1`, `delay`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`)
VALUES (90215, 2, 15, "Mind Render's Dagger", 12993,
    4, 0, 13, -1,
    35, 25, 1,
    35, 65, 6, 1800,
    3, 12, 7, 10);

-- 90216 Aku'mai's Tentacle (Off-hand mace, Aku'mai drop)
INSERT INTO `item_template` (`entry`, `class`, `subclass`, `name`, `displayid`,
    `Quality`, `Flags`, `InventoryType`, `AllowableClass`,
    `ItemLevel`, `RequiredLevel`, `bonding`,
    `dmg_min1`, `dmg_max1`, `dmg_type1`, `delay`,
    `stat_type1`, `stat_value1`, `stat_type2`, `stat_value2`)
VALUES (90216, 2, 4, "Aku'mai's Tentacle", 4555,
    4, 0, 22, -1,
    36, 25, 1,
    40, 70, 0, 2000,
    4, 14, 7, 10);

-- ============================================================================
-- Wire each new item into bracket_personal_loot at 2% chance per kill
-- ============================================================================
-- 4 = Strength, 5 = Intellect; chance lower than dungeon-tier (4%) to
-- preserve scarcity.

DELETE FROM `bracket_personal_loot` WHERE `item_id` BETWEEN 90200 AND 90216;
INSERT INTO `bracket_personal_loot` (`boss_entry`, `item_id`, `chance`, `comment`) VALUES
    -- Ghamoo-ra (4887)
    (4887, 90200, 2, 'Tidesworn Pauldrons (tier)'),
    (4887, 90210, 2, 'Adamant Maul'),
    -- Lady Sarevess (4831)
    (4831, 90201, 2, 'Tidesworn Robes (tier)'),
    (4831, 90211, 2, "Naga Sea-Witch's Wand"),
    -- Old Serra'kis (4830, used as Baron Aquanis slot)
    (4830, 90202, 2, 'Tidesworn Hauberk (tier)'),
    (4830, 90212, 2, 'Razor-Fin Tooth Necklace'),
    -- Gelihast (6243)
    (6243, 90203, 2, 'Tidesworn Leggings (tier)'),
    (6243, 90213, 2, "Murloc Slayer's Mitts"),
    -- Lorgus Jett (207356)
    (207356, 90204, 2, 'Tidesworn Helm (tier)'),
    (207356, 90214, 2, 'Twilight Cult Libram'),
    -- Twilight Lord Kelris (4832)
    (4832, 90205, 2, 'Tidesworn Headcover (tier)'),
    (4832, 90215, 2, "Mind Render's Dagger"),
    -- Aku'mai (4829)
    (4829, 90206, 2, 'Tidesworn Chestguard (tier)'),
    (4829, 90216, 2, "Aku'mai's Tentacle");
