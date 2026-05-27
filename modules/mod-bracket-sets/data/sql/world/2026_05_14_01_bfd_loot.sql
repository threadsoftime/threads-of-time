--
-- mod-bracket-sets — Bracket 1 loot integration (BFD-only, Epic quality)
--
-- 1. Bump all 32 itemset=90101 items to Epic (Quality=4). Previously 31 were
--    Rare (3); only 90051 was already Epic.
-- 2. Drop only from BFD (map 48) bosses. Prior state had 52 broken loot rows
--    pointing to a Shadowfang Keep mob (Sever, map 33) and a handful of
--    gameobject/orphan IDs in the wrong loot table -- effectively 30/32
--    items had no working drop source.
--
-- BFD bosses (stock AC, all map 48, rank=1):
--   4887 Ghamoo-ra              (entrance turtle)
--   4831 Lady Sarevess          (naga caster)
--   4830 Old Serra'kis          (optional shark)
--   6243 Gelihast               (optional Twilight murloc)
--   4832 Twilight Lord Kelris   (cult phase boss)
--   4829 Aku'mai                (final boss)
--
-- Distribution: 5/6/5/5/5/6 with light thematic mapping where item name
-- matches boss (e.g., Ghamoo-ra's Cinch -> Ghamoo-ra).
--
-- Drop chance: 20% per item per kill. With 5-6 items per boss this yields
-- ~1 item per kill on average under stock WotLK loot rules. Phase 3
-- (personal-loot script) will replace these rows with its own table.
--

-- ============================================================================
-- 1. Epic quality bump
-- ============================================================================
UPDATE `item_template` SET `Quality` = 4 WHERE `itemset` = 90101;

-- ============================================================================
-- 2. Wipe the prior broken loot rows
-- ============================================================================
DELETE FROM `creature_loot_template` WHERE `Item` BETWEEN 90000 AND 90100;

-- ============================================================================
-- 3. Insert BFD boss drops
-- ============================================================================
-- Columns: Entry (creature_template.entry), Item (item_template.entry),
--          Chance, QuestRequiredForDrop, LootMode, GroupId, MinCount, MaxCount

-- Ghamoo-ra (4887) -- turtle / aquatic / faithful: 5 items
INSERT INTO `creature_loot_template`
    (`Entry`, `Item`, `Reference`, `Chance`, `QuestRequired`, `LootMode`, `GroupId`, `MinCount`, `MaxCount`, `Comment`)
VALUES
    (4887, 90000, 0, 20, 0, 1, 0, 1, 1, 'Hydraxian Bangles'),
    (4887, 90002, 0, 20, 0, 1, 0, 1, 1, 'Adamantine Tortoise Armor'),
    (4887, 90006, 0, 20, 0, 1, 0, 1, 1, 'Shell Plate Barrier'),
    (4887, 90007, 0, 20, 0, 1, 0, 1, 1, "Ghamoo-ra's Cinch"),
    (4887, 90037, 0, 20, 0, 1, 0, 1, 1, 'Cracked Water Globe');

-- Lady Sarevess (4831) -- naga caster: 6 items
INSERT INTO `creature_loot_template`
    (`Entry`, `Item`, `Reference`, `Chance`, `QuestRequired`, `LootMode`, `GroupId`, `MinCount`, `MaxCount`, `Comment`)
VALUES
    (4831, 90004, 0, 20, 0, 1, 0, 1, 1, 'High Tide Choker'),
    (4831, 90005, 0, 20, 0, 1, 0, 1, 1, 'Flowing Scarf'),
    (4831, 90012, 0, 20, 0, 1, 0, 1, 1, 'Naga Battle Gauntlets'),
    (4831, 90021, 0, 20, 0, 1, 0, 1, 1, 'Leggings of the Faithful'),
    (4831, 90025, 0, 20, 0, 1, 0, 1, 1, 'Tome of Cavern Lore'),
    (4831, 90036, 0, 20, 0, 1, 0, 1, 1, 'Silver Hand Sabatons');

-- Old Serra'kis (4830) -- shark / thresher: 5 items
INSERT INTO `creature_loot_template`
    (`Entry`, `Item`, `Reference`, `Chance`, `QuestRequired`, `LootMode`, `GroupId`, `MinCount`, `MaxCount`, `Comment`)
VALUES
    (4830, 90009, 0, 20, 0, 1, 0, 1, 1, 'Shimmering Thresher Cape'),
    (4830, 90010, 0, 20, 0, 1, 0, 1, 1, "Bindings of Serra'kis"),
    (4830, 90020, 0, 20, 0, 1, 0, 1, 1, 'Band of Deep Places'),
    (4830, 90027, 0, 20, 0, 1, 0, 1, 1, 'Black Boiled Leathers'),
    (4830, 90049, 0, 20, 0, 1, 0, 1, 1, 'Mantle of the Thresher Slayer');

-- Gelihast (6243) -- Twilight murloc: 5 items
INSERT INTO `creature_loot_template`
    (`Entry`, `Item`, `Reference`, `Chance`, `QuestRequired`, `LootMode`, `GroupId`, `MinCount`, `MaxCount`, `Comment`)
VALUES
    (6243, 90023, 0, 20, 0, 1, 0, 1, 1, 'Algae Gauntlets'),
    (6243, 90024, 0, 20, 0, 1, 0, 1, 1, 'Murloc Hide Kneeboots'),
    (6243, 90043, 0, 20, 0, 1, 0, 1, 1, 'Black Fingerless Gloves'),
    (6243, 90044, 0, 20, 0, 1, 0, 1, 1, 'Glowing Fetish Amulet'),
    (6243, 90046, 0, 20, 0, 1, 0, 1, 1, 'Clamweave Tunic');

-- Twilight Lord Kelris (4832) -- cult phase boss: 5 items
INSERT INTO `creature_loot_template`
    (`Entry`, `Item`, `Reference`, `Chance`, `QuestRequired`, `LootMode`, `GroupId`, `MinCount`, `MaxCount`, `Comment`)
VALUES
    (4832, 90038, 0, 20, 0, 1, 0, 1, 1, 'Gaze Dreamer Leggings'),
    (4832, 90039, 0, 20, 0, 1, 0, 1, 1, 'Signet of the Twilight Lord'),
    (4832, 90040, 0, 20, 0, 1, 0, 1, 1, "Twilight Invoker's Shoes"),
    (4832, 90042, 0, 20, 0, 1, 0, 1, 1, "Twilight Invoker's Robes"),
    (4832, 90051, 0, 20, 0, 1, 0, 1, 1, 'Abomination Skin Leggings');

-- Aku'mai (4829) -- final boss, sea horror: 6 items
INSERT INTO `creature_loot_template`
    (`Entry`, `Item`, `Reference`, `Chance`, `QuestRequired`, `LootMode`, `GroupId`, `MinCount`, `MaxCount`, `Comment`)
VALUES
    (4829, 90003, 0, 20, 0, 1, 0, 1, 1, 'Cord of Aquanis'),
    (4829, 90030, 0, 20, 0, 1, 0, 1, 1, 'Carved Driftwood Icon'),
    (4829, 90033, 0, 20, 0, 1, 0, 1, 1, 'Glowing Leather Bands'),
    (4829, 90041, 0, 20, 0, 1, 0, 1, 1, 'Skinwalkers'),
    (4829, 90047, 0, 20, 0, 1, 0, 1, 1, 'Shoulderguards of Crushing Depths'),
    (4829, 90048, 0, 20, 0, 1, 0, 1, 1, 'Loop of Swift Currents');
