--
-- mod-bracket-sets — bracket_set_bonus_map DDL
--
-- Lookup table for per-class+spec tier-set bonus spells.
-- Indexed by (itemset_id, threshold, class_id, spec_id). The mod's PlayerScript
-- queries this on every re-eval (login, equip, level-change, spec-change).
--

CREATE TABLE IF NOT EXISTS `bracket_set_bonus_map` (
  `itemset_id`   INT UNSIGNED     NOT NULL                          COMMENT 'AzerothCore item_set.id (90101 plate, 90102 mail, 90103 leather, 90104 cloth for Bracket 1)',
  `threshold`    TINYINT UNSIGNED NOT NULL                          COMMENT '2 (2-piece bonus) or 4 (4-piece bonus)',
  `class_id`     TINYINT UNSIGNED NOT NULL                          COMMENT '1 Warrior, 2 Paladin, 3 Hunter, 4 Rogue, 5 Priest, 6 DK, 7 Shaman, 8 Mage, 9 Warlock, 11 Druid',
  `spec_id`      TINYINT UNSIGNED NOT NULL                          COMMENT 'Talent tree index 1, 2, or 3 (per class enum in mod)',
  `spell_id`     INT UNSIGNED     NOT NULL                          COMMENT 'Spell.dbc marker spell whose AuraScript implements the bonus',
  `bracket_min`  TINYINT UNSIGNED NOT NULL                          COMMENT '25 for Bracket 1',
  `bracket_max`  TINYINT UNSIGNED NOT NULL                          COMMENT '34 for Bracket 1',
  `display_name` VARCHAR(128)     NOT NULL DEFAULT ''               COMMENT 'Bonus display name for tooltip / debug',
  PRIMARY KEY (`itemset_id`, `threshold`, `class_id`, `spec_id`),
  KEY `idx_itemset` (`itemset_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
  COMMENT='Phase 1.5 — bracket-locked tier set bonus dispatch table';
