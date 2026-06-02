-- v0.2: discriminate memory shapes (event/fact/goal_link) and track
-- where each memory came from. Both additive; existing rows get the
-- default value for memory_type and NULL for source.
ALTER TABLE memories ADD COLUMN memory_type TEXT NOT NULL DEFAULT 'event';
ALTER TABLE memories ADD COLUMN source      TEXT;

-- Helpful indexes for the new memory.list / memory.search filters.
CREATE INDEX IF NOT EXISTS idx_memories_bot_type    ON memories(bot_id, memory_type);
CREATE INDEX IF NOT EXISTS idx_memories_bot_created ON memories(bot_id, created_ts);
