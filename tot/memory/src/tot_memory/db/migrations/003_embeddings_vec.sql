-- SPDX-License-Identifier: GPL-2.0-or-later
-- Migration: 003_embeddings_vec.sql
-- Per design subspec §3 (sqlite-vec Virtual Table).
--
-- Dense vector store via sqlite-vec.
-- Dimension MUST match EMBEDDING_DIM in tot_memory/db/schema.py.
-- For nomic-embed-text: 768 (see §4 for model selection rationale).
--
-- Convention: rowid in embeddings_vec = episode_id in episodes. The write
-- route inserts (episode_id, embedding_bytes) and sets
-- episodes.content_embedding_id = episode_id, making the join trivial.

CREATE VIRTUAL TABLE embeddings_vec USING vec0(
    embedding float[768]
);
