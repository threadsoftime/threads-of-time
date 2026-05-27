# SPDX-License-Identifier: GPL-2.0-or-later
"""Entity hard-filter for hybrid recall.

Per design subspec §2 / §6: the recall caller can restrict the candidate pool
to episodes that reference one or more named entities. The filter is OR-logic
across the supplied names — any matching episode passes — and the entity name
match is exact against ``entities.display_name``.

This module returns the *set of matching episode_ids*. The orchestrator in
:mod:`tot_memory.retrieval.rerank` intersects this set with the BM25 + dense
candidate set before scoring.
"""

import sqlite3


def entity_filter(conn: sqlite3.Connection, entity_names: list[str]) -> set[int]:
    """Return ``episode_id`` values referencing any entity whose
    ``display_name`` is in ``entity_names``.

    Empty input (or no matches) → empty set. The empty set is a sentinel the
    caller may treat as "no filter requested" *or* "filter requested but
    matched nothing"; the rerank orchestrator distinguishes these cases by
    only invoking ``entity_filter`` when ``entity_names`` is non-empty.

    Uses the §2.1 schema join:
        episodes ─ episode_entities ─ entities (display_name IN ?)
    """
    if not entity_names:
        return set()

    placeholders = ",".join("?" * len(entity_names))
    rows = conn.execute(
        f"SELECT DISTINCT ee.episode_id "
        f"FROM episode_entities AS ee "
        f"JOIN entities AS e ON e.entity_id = ee.entity_id "
        f"WHERE e.display_name IN ({placeholders})",
        tuple(entity_names),
    ).fetchall()
    return {int(r[0]) for r in rows}
