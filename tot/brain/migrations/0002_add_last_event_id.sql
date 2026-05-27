-- Migration 0002: add last_event_id to living_bots for SSE reconnect cursor.
-- Applied exactly once by StateStore.migrate() which guards on schema_version.
ALTER TABLE living_bots ADD COLUMN last_event_id INTEGER NOT NULL DEFAULT 0;
INSERT OR IGNORE INTO schema_version (version) VALUES (2);
