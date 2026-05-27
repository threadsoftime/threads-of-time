CREATE TABLE IF NOT EXISTS living_bots (
    bot_guid INTEGER PRIMARY KEY,
    enrolled_at INTEGER NOT NULL,
    last_seen INTEGER,
    status TEXT NOT NULL CHECK (status IN ('active','paused','released','error_no_personality')),
    personality_seed_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS decisions_recent (
    bot_guid INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    ts_ms INTEGER NOT NULL,
    decision_json TEXT NOT NULL,
    PRIMARY KEY (bot_guid, seq)
);

CREATE INDEX IF NOT EXISTS idx_decisions_recent_ts
    ON decisions_recent(bot_guid, ts_ms);

CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY
);

INSERT OR IGNORE INTO schema_version (version) VALUES (1);
