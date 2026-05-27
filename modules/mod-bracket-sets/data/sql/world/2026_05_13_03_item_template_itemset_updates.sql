--
-- mod-bracket-sets — assign Phase 1.0 Bracket 1 items to itemset 90101
--
-- 32 items get itemset = 90101 (Vestiges of Blackfathom):
--   - 8 Cloth   (class=4, subclass=1)
--   - 7 Leather (class=4, subclass=2)
--   - 6 Mail    (class=4, subclass=3)
--   - 0 Plate   (class=4, subclass=4) — none imported in Phase 1.0
--   - 1 Buckler (class=4, subclass=5)
--   - 10 Misc   (class=4, subclass=0) — rings/necks/cloaks/trinkets
--
-- Excluded from the set (20 items):
--   - Weapons   (class=2, 18 items): off-set, no bonus contribution
--   - Shields   (class=4, subclass=6, 2 items): off-set
--   - Libram    (class=4, subclass=7, 1 item):  off-set (Paladin relic, Mind-Expanding Mushroom)
--

-- Cloth (8 items)
UPDATE `item_template` SET `itemset` = 90101 WHERE `entry` IN
  (90007, 90038, 90040, 90041, 90042, 90043, 90046, 90051);

-- Leather (7 items)
UPDATE `item_template` SET `itemset` = 90101 WHERE `entry` IN
  (90003, 90010, 90012, 90024, 90027, 90033, 90049);

-- Mail (6 items)
UPDATE `item_template` SET `itemset` = 90101 WHERE `entry` IN
  (90000, 90002, 90021, 90023, 90036, 90047);

-- Buckler (1 item) — included as wearable off-hand armor
UPDATE `item_template` SET `itemset` = 90101 WHERE `entry` IN
  (90006);

-- Misc armor: necks/cloaks/rings/trinkets (10 items) — wearable by any class
UPDATE `item_template` SET `itemset` = 90101 WHERE `entry` IN
  (90004, 90005, 90009, 90020, 90025, 90030, 90037, 90039, 90044, 90048);

-- Verification: should return 32 rows
-- SELECT COUNT(*) FROM item_template WHERE entry BETWEEN 90000 AND 90100 AND itemset = 90101;
