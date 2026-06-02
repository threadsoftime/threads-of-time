-- v0.2: FTS5 virtual table + sync triggers + initial bulk load.
-- Idempotent — bulk-load INSERT is guarded by NOT IN so re-running
-- this migration on a populated FTS does nothing. Spec §4.5.

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    text,
    content='memories',
    content_rowid='rowid',
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS memories_fts_insert
    AFTER INSERT ON memories BEGIN
        INSERT INTO memories_fts(rowid, text) VALUES (new.rowid, new.text);
    END;

CREATE TRIGGER IF NOT EXISTS memories_fts_delete
    AFTER DELETE ON memories BEGIN
        INSERT INTO memories_fts(memories_fts, rowid, text)
            VALUES('delete', old.rowid, old.text);
    END;

CREATE TRIGGER IF NOT EXISTS memories_fts_update
    AFTER UPDATE OF text ON memories BEGIN
        INSERT INTO memories_fts(memories_fts, rowid, text)
            VALUES('delete', old.rowid, old.text);
        INSERT INTO memories_fts(rowid, text) VALUES (new.rowid, new.text);
    END;

-- Initial bulk load. Idempotent via NOT IN guard.
INSERT INTO memories_fts(rowid, text)
    SELECT rowid, text FROM memories
    WHERE rowid NOT IN (SELECT rowid FROM memories_fts);
