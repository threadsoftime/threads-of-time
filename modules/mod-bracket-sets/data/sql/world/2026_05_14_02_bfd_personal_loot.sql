--
-- mod-bracket-sets — Phase 3: personal loot for BFD bosses.
--
-- Items 90000-90100 now drop ONLY via the per-player roll in
-- BracketSetsPersonalLoot.cpp; stock creature_loot_template rows are
-- removed so they don't double-drop on the corpse.
--
-- bracket_personal_loot is the canonical source for the C++ script:
-- each row is a (boss, item, chance) tuple where `chance` is the percent
-- per player per kill. 4% is the default and yields ~1-2 drops per
-- 5-man clear and ~2-4 per 10-man.
--

DROP TABLE IF EXISTS `bracket_personal_loot`;
CREATE TABLE `bracket_personal_loot` (
  `boss_entry` INT UNSIGNED NOT NULL COMMENT 'creature_template.entry of the boss',
  `item_id`    INT UNSIGNED NOT NULL COMMENT 'item_template.entry of the loot item',
  `chance`     FLOAT        NOT NULL DEFAULT 4 COMMENT 'percent chance per player per kill',
  `comment`    VARCHAR(255)          DEFAULT NULL,
  PRIMARY KEY (`boss_entry`, `item_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
  COMMENT='mod-bracket-sets: per-player loot rolls for BFD bosses';

-- Ghamoo-ra (4887) — turtle / aquatic theme: 5 items
INSERT INTO `bracket_personal_loot` (`boss_entry`, `item_id`, `chance`, `comment`) VALUES
    (4887, 90000, 4, 'Hydraxian Bangles'),
    (4887, 90002, 4, 'Adamantine Tortoise Armor'),
    (4887, 90006, 4, 'Shell Plate Barrier'),
    (4887, 90007, 4, "Ghamoo-ra's Cinch"),
    (4887, 90037, 4, 'Cracked Water Globe');

-- Lady Sarevess (4831) — naga caster: 6 items
INSERT INTO `bracket_personal_loot` (`boss_entry`, `item_id`, `chance`, `comment`) VALUES
    (4831, 90004, 4, 'High Tide Choker'),
    (4831, 90005, 4, 'Flowing Scarf'),
    (4831, 90012, 4, 'Naga Battle Gauntlets'),
    (4831, 90021, 4, 'Leggings of the Faithful'),
    (4831, 90025, 4, 'Tome of Cavern Lore'),
    (4831, 90036, 4, 'Silver Hand Sabatons');

-- Old Serra'kis (4830) — shark / thresher: 5 items
INSERT INTO `bracket_personal_loot` (`boss_entry`, `item_id`, `chance`, `comment`) VALUES
    (4830, 90009, 4, 'Shimmering Thresher Cape'),
    (4830, 90010, 4, "Bindings of Serra'kis"),
    (4830, 90020, 4, 'Band of Deep Places'),
    (4830, 90027, 4, 'Black Boiled Leathers'),
    (4830, 90049, 4, 'Mantle of the Thresher Slayer');

-- Gelihast (6243) — Twilight murloc: 5 items
INSERT INTO `bracket_personal_loot` (`boss_entry`, `item_id`, `chance`, `comment`) VALUES
    (6243, 90023, 4, 'Algae Gauntlets'),
    (6243, 90024, 4, 'Murloc Hide Kneeboots'),
    (6243, 90043, 4, 'Black Fingerless Gloves'),
    (6243, 90044, 4, 'Glowing Fetish Amulet'),
    (6243, 90046, 4, 'Clamweave Tunic');

-- Twilight Lord Kelris (4832) — cult phase boss: 5 items
INSERT INTO `bracket_personal_loot` (`boss_entry`, `item_id`, `chance`, `comment`) VALUES
    (4832, 90038, 4, 'Gaze Dreamer Leggings'),
    (4832, 90039, 4, 'Signet of the Twilight Lord'),
    (4832, 90040, 4, "Twilight Invoker's Shoes"),
    (4832, 90042, 4, "Twilight Invoker's Robes"),
    (4832, 90051, 4, 'Abomination Skin Leggings');

-- Aku'mai (4829) — final boss, sea horror: 6 items
INSERT INTO `bracket_personal_loot` (`boss_entry`, `item_id`, `chance`, `comment`) VALUES
    (4829, 90003, 4, 'Cord of Aquanis'),
    (4829, 90030, 4, 'Carved Driftwood Icon'),
    (4829, 90033, 4, 'Glowing Leather Bands'),
    (4829, 90041, 4, 'Skinwalkers'),
    (4829, 90047, 4, 'Shoulderguards of Crushing Depths'),
    (4829, 90048, 4, 'Loop of Swift Currents');

-- Remove stock creature_loot_template rows for these items so they
-- only drop via the personal-loot script. (Phase 2's INSERT created
-- 32 rows; this Phase 3 step retires them.)
DELETE FROM `creature_loot_template` WHERE `Item` BETWEEN 90000 AND 90100;
