--
-- mod-bracket-sets — bracket_set_itemset_map seed (Bracket 1)
--
-- 27 rows: 9 classes x 3 specs. Maps each class+spec to a patched
-- ItemSet.dbc row in the client's patch-Z.MPQ.
--
-- Numbering scheme 901CS where C = contiguous class slot (1-9), S = spec (1-3).
-- See spec §6 for full mapping.
--

DELETE FROM `bracket_set_itemset_map` WHERE `bracket_id` = 1;

INSERT INTO `bracket_set_itemset_map` (`class_id`, `spec_id`, `bracket_id`, `itemset_id`) VALUES
  -- Warrior (class 1)
  ( 1, 1, 1, 90111),  -- Arms
  ( 1, 2, 1, 90112),  -- Fury
  ( 1, 3, 1, 90113),  -- Prot
  -- Paladin (class 2)
  ( 2, 1, 1, 90121),  -- Holy
  ( 2, 2, 1, 90122),  -- Prot
  ( 2, 3, 1, 90123),  -- Ret
  -- Hunter (class 3)
  ( 3, 1, 1, 90131),  -- BM
  ( 3, 2, 1, 90132),  -- MM
  ( 3, 3, 1, 90133),  -- Surv
  -- Rogue (class 4)
  ( 4, 1, 1, 90141),  -- Assn
  ( 4, 2, 1, 90142),  -- Combat
  ( 4, 3, 1, 90143),  -- Subt
  -- Priest (class 5)
  ( 5, 1, 1, 90151),  -- Disc
  ( 5, 2, 1, 90152),  -- Holy
  ( 5, 3, 1, 90153),  -- Shadow
  -- Shaman (class 7)
  ( 7, 1, 1, 90161),  -- Ele
  ( 7, 2, 1, 90162),  -- Enh
  ( 7, 3, 1, 90163),  -- Resto
  -- Mage (class 8)
  ( 8, 1, 1, 90171),  -- Arc
  ( 8, 2, 1, 90172),  -- Fire
  ( 8, 3, 1, 90173),  -- Frost
  -- Warlock (class 9)
  ( 9, 1, 1, 90181),  -- Aff
  ( 9, 2, 1, 90182),  -- Demo
  ( 9, 3, 1, 90183),  -- Destr
  -- Druid (class 11)
  (11, 1, 1, 90191),  -- Balance
  (11, 2, 1, 90192),  -- Feral
  (11, 3, 1, 90193);  -- Resto

-- Verification: should return 27
-- SELECT COUNT(*) FROM bracket_set_itemset_map WHERE bracket_id = 1;
