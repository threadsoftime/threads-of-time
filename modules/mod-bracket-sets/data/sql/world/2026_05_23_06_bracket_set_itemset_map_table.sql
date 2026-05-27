--
-- mod-bracket-sets — bracket_set_itemset_map DDL
--
-- Lookup table mapping (class_id, spec_id, bracket_id) to the patched
-- client-side itemset_dbc row that should be sent in SMSG_ITEM_QUERY_SINGLE_RESPONSE.
-- Loaded once at worldserver startup by mod-bracket-sets into an
-- in-memory std::map. No per-query DB hit; resolver is O(1).
--
-- See: docs/superpowers/specs/2026-05-23-bracket1-tier-set-ui-client-patch-design.md §5.1
--

CREATE TABLE IF NOT EXISTS `bracket_set_itemset_map` (
  `class_id`   TINYINT UNSIGNED NOT NULL  COMMENT '1 Warr, 2 Pal, 3 Hunt, 4 Rog, 5 Pri, 7 Sha, 8 Mage, 9 Lock, 11 Druid',
  `spec_id`    TINYINT UNSIGNED NOT NULL  COMMENT 'Talent tree 1/2/3',
  `bracket_id` TINYINT UNSIGNED NOT NULL  COMMENT '1 for Bracket 1; scales to 2-7 later',
  `itemset_id` INT UNSIGNED     NOT NULL  COMMENT 'Patched client-side itemset_dbc row (90111..90193 for Bracket 1)',
  PRIMARY KEY (`class_id`, `spec_id`, `bracket_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
  COMMENT='Maps class+spec to patched itemset_dbc row for client tooltip rendering (spec 2026-05-23)';
