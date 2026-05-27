# SPDX-License-Identifier: GPL-2.0-or-later
"""Hybrid retrieval engine for the V3 memory subsystem.

Per design subspec §6, recall combines:

- BM25 keyword search over ``episodes_fts`` (``bm25.py``)
- Dense vector KNN over ``embeddings_vec`` (``dense.py``)
- Optional entity hard filter (``entity.py``)
- Per-episode-type time decay (``decay.py``)
- The hybrid scoring formula α·bm25 + β·dense, multiplied by decay and the
  salience boost ``(1 + γ·salience)``, plus an additive entity-match bonus δ
  (``hybrid.py``)
- The orchestrator that pulls candidates from each surface, applies the filter,
  hydrates ``episodes`` rows, scores, sorts, and returns top-K (``rerank.py``)
"""
