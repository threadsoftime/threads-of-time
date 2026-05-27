--
-- mod-rotation-mode — heimbot_settings DDL
--
-- Per-character key-value store for PE2 settings. Lives in acore_characters
-- (per-character data domain). Defaults live in code in mod-rotation-mode, so
-- an empty table for a given character == vanilla behavior.
--
-- This file is auto-applied by AzerothCore's module-SQL discovery at boot
-- (or manually via scripts/apply-sql.sh).
--

CREATE TABLE IF NOT EXISTS `heimbot_settings` (
  `character_guid`  BIGINT UNSIGNED  NOT NULL,
  `setting_key`     VARCHAR(64)      NOT NULL,
  `setting_value`   VARCHAR(256)     NOT NULL,
  `updated_at`      TIMESTAMP        NOT NULL
                                     DEFAULT CURRENT_TIMESTAMP
                                     ON UPDATE CURRENT_TIMESTAMP,
  PRIMARY KEY (`character_guid`, `setting_key`),
  KEY `idx_character` (`character_guid`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
  COMMENT='Per-character heimbot (rotation/grind/squad-mirror) settings';
