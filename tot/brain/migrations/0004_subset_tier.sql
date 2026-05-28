-- 0004_subset_tier.sql — Plan 3 subset-gating columns
-- SPDX-License-Identifier: GPL-2.0-or-later

ALTER TABLE living_bots ADD COLUMN tier TEXT NOT NULL DEFAULT 'full';
ALTER TABLE living_bots ADD COLUMN out_of_range_ticks INTEGER NOT NULL DEFAULT 0;
ALTER TABLE living_bots ADD COLUMN in_range_ticks INTEGER NOT NULL DEFAULT 0;
ALTER TABLE living_bots ADD COLUMN last_recompute_at INTEGER;
ALTER TABLE living_bots ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_living_bots_tier ON living_bots(status, tier);

INSERT OR IGNORE INTO schema_version(version) VALUES (4);
