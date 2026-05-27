-- SPDX-License-Identifier: GPL-2.0-or-later
-- Migration: 001_episodes.sql
-- Per design subspec §1 (Episode Schema).

CREATE TABLE episodes (
    episode_id          INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp           INTEGER NOT NULL,          -- unix epoch ms (event time, NOT write time)
    created_at          INTEGER NOT NULL            -- unix epoch ms (row write time)
                        DEFAULT (unixepoch('now') * 1000),
    content_text        TEXT    NOT NULL            -- the memory text; max ~4000 chars (seq-len limit)
                        CHECK  (length(content_text) > 0 AND length(content_text) <= 4000),
    content_embedding_id INTEGER,                   -- rowid in embeddings_vec; NULL until embedded
    episode_type        TEXT    NOT NULL            -- see §1.2 for valid values
                        CHECK  (episode_type IN (
                            'chat',        -- player/bot said something to the bot
                            'combat',      -- notable combat event (kill, near-death, group wipe)
                            'social',      -- non-combat interaction with a player or NPC
                            'quest',       -- quest accept/complete/fail/objective
                            'discovery',   -- new place, item, mechanic discovered
                            'goal',        -- goal proposed, adopted, completed, or abandoned
                            'reflection',  -- bot-generated synthesis: distilled from other episodes
                            'observation'  -- passive observation (player joined, someone died nearby)
                        )),
    salience_score      REAL    NOT NULL DEFAULT 0.5
                        CHECK  (salience_score >= 0.0 AND salience_score <= 1.0),
    last_recalled_at    INTEGER,                    -- unix epoch ms; NULL = never recalled
    recall_count        INTEGER NOT NULL DEFAULT 0
                        CHECK  (recall_count >= 0),
    source              TEXT    NOT NULL DEFAULT 'self'
                        CHECK  (source IN ('self', 'chat', 'observed', 'system')),
    metadata            TEXT                        -- JSON blob; arbitrary per-episode-type fields
);

-- Primary access patterns: bot's episodes by recency, by type
CREATE INDEX ix_episodes_timestamp        ON episodes(timestamp DESC);
CREATE INDEX ix_episodes_episode_type     ON episodes(episode_type, timestamp DESC);
CREATE INDEX ix_episodes_salience         ON episodes(salience_score DESC);
CREATE INDEX ix_episodes_last_recalled    ON episodes(last_recalled_at DESC);
CREATE INDEX ix_episodes_embedding        ON episodes(content_embedding_id)
    WHERE content_embedding_id IS NOT NULL;
