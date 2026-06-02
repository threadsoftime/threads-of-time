-- v0.2: goals table (R3 from kb_2b0f0aa4). Status lifecycle enforced
-- in API layer (routes_goals.py). origin_memory is a SOFT FK (no
-- constraint) so a memory deletion leaves dangling references rather
-- than cascading. Spec §4.3.

CREATE TABLE IF NOT EXISTS goals (
    id            TEXT PRIMARY KEY,
    bot_id        TEXT NOT NULL,
    text          TEXT NOT NULL,
    status        TEXT NOT NULL,
    source        TEXT,
    priority      INTEGER NOT NULL DEFAULT 0,
    origin_memory TEXT,
    created_ts    INTEGER NOT NULL,
    updated_ts    INTEGER NOT NULL,
    completed_ts  INTEGER
);

CREATE INDEX IF NOT EXISTS idx_goals_bot_status   ON goals(bot_id, status);
CREATE INDEX IF NOT EXISTS idx_goals_bot_priority ON goals(bot_id, priority DESC);
