-- v0.1 baseline schema — idempotent re-statement of what's already in
-- production. New installs will create everything from here; existing
-- prod DBs will INSERT into schema_version with version=1 marking that
-- the baseline is established.

CREATE TABLE IF NOT EXISTS bots (
    bot_id     TEXT PRIMARY KEY,
    persona    TEXT,
    created_ts INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS entities (
    id           INTEGER PRIMARY KEY,
    bot_id       TEXT NOT NULL,
    name_lower   TEXT NOT NULL,
    display_name TEXT NOT NULL,
    type         TEXT,
    UNIQUE(bot_id, name_lower)
);

CREATE TABLE IF NOT EXISTS edges (
    src_entity_id INTEGER NOT NULL,
    rel           TEXT NOT NULL,
    dst_entity_id INTEGER NOT NULL,
    weight        REAL DEFAULT 1.0,
    last_seen_ts  INTEGER NOT NULL,
    PRIMARY KEY (src_entity_id, rel, dst_entity_id)
);

CREATE TABLE IF NOT EXISTS memories (
    id                TEXT PRIMARY KEY,
    bot_id            TEXT NOT NULL,
    text              TEXT NOT NULL,
    salience          REAL NOT NULL,
    created_ts        INTEGER NOT NULL,
    last_recalled_ts  INTEGER NOT NULL,
    embedding         BLOB
);

CREATE TABLE IF NOT EXISTS memory_entities (
    memory_id TEXT NOT NULL,
    entity_id INTEGER NOT NULL,
    PRIMARY KEY (memory_id, entity_id)
);

CREATE INDEX IF NOT EXISTS idx_memories_bot ON memories(bot_id);
CREATE INDEX IF NOT EXISTS idx_entities_bot_name ON entities(bot_id, name_lower);

-- sqlite-vec virtual table; safe to re-declare with IF NOT EXISTS.
CREATE VIRTUAL TABLE IF NOT EXISTS vec_memories USING vec0(
    memory_id TEXT PRIMARY KEY,
    bot_id    TEXT,
    embedding FLOAT[384]
);
