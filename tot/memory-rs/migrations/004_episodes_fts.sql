-- SPDX-License-Identifier: GPL-2.0-or-later
-- Migration: 004_episodes_fts.sql
-- Per design subspec §6 (Hybrid Retrieval Algorithm — BM25 component).
--
-- FTS5 virtual table over episodes.content_text, with the porter stemmer
-- over unicode61 tokenization (standard English-language FTS5 choice;
-- design subspec §6 specifies BM25 + FTS5 without naming a tokenizer,
-- so we pick the conventional default).
--
-- contentless-external pattern: the FTS index doesn't store its own copy
-- of content_text; it references the row in `episodes` via content_rowid.
-- AFTER INSERT/DELETE/UPDATE triggers keep the index in sync.

CREATE VIRTUAL TABLE episodes_fts USING fts5(
    content_text,
    content='episodes',
    content_rowid='episode_id',
    tokenize='porter unicode61'
);

CREATE TRIGGER episodes_ai AFTER INSERT ON episodes BEGIN
    INSERT INTO episodes_fts(rowid, content_text)
    VALUES (new.episode_id, new.content_text);
END;

CREATE TRIGGER episodes_ad AFTER DELETE ON episodes BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, content_text)
    VALUES ('delete', old.episode_id, old.content_text);
END;

CREATE TRIGGER episodes_au AFTER UPDATE OF content_text ON episodes BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, content_text)
    VALUES ('delete', old.episode_id, old.content_text);
    INSERT INTO episodes_fts(rowid, content_text)
    VALUES (new.episode_id, new.content_text);
END;
