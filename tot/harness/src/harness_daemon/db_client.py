"""Daemon-direct MySQL queries for obs.query_db.

Templates are pre-registered (no free-form SQL). Each template declares
its parameter names and produces a parameterized statement.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import aiomysql


class UnknownTemplate(KeyError):
    pass


class BadParams(ValueError):
    pass


@dataclass(frozen=True)
class QueryTemplate:
    name:      str
    sql:       str
    db:        str           # "acore_world", "acore_characters", "acore_auth"
    params:    list[str]     # ordered param names


# V1 templates — extend as smoke scenarios require.
V1_TEMPLATES: dict[str, QueryTemplate] = {
    "bracket_set_bonus_map_for": QueryTemplate(
        name="bracket_set_bonus_map_for",
        db="acore_world",
        # Column is `spell_id` per modules/mod-bracket-sets/data/sql/world/
        # 2026_05_13_00_bracket_set_bonus_map_table.sql (it's the Spell.dbc
        # marker whose AuraScript implements the bonus). bracket_min/max +
        # display_name are returned for context (smoke tests use display_name
        # for assertion messages; bracket_min/max gate level-range checks).
        sql=(
            "SELECT itemset_id, threshold, class_id, spec_id, spell_id, "
            "       bracket_min, bracket_max, display_name "
            "FROM bracket_set_bonus_map "
            "WHERE itemset_id=%s AND class_id=%s AND spec_id=%s "
            "ORDER BY threshold"
        ),
        params=["itemset_id", "class_id", "spec_id"],
    ),
    "item_template_set_pieces": QueryTemplate(
        name="item_template_set_pieces",
        db="acore_world",
        sql=(
            "SELECT entry, name, ItemSet, class, subclass, quality, "
            "       InventoryType, RequiredLevel "
            "FROM item_template "
            "WHERE ItemSet=%s "
            "ORDER BY entry"
        ),
        params=["itemset_id"],
    ),
    "bracketsets_diag": QueryTemplate(
        name="bracketsets_diag",
        db="acore_world",
        sql=(
            "SELECT spell_id, ScriptName "
            "FROM spell_script_names "
            "WHERE spell_id=%s"
        ),
        params=["spell_id"],
    ),
    "character_online": QueryTemplate(
        name="character_online",
        db="acore_characters",
        sql=(
            "SELECT guid, name, race, class, level, online "
            "FROM characters "
            "WHERE guid=%s"
        ),
        params=["guid"],
    ),
    # Smoke tests use this to pick an existing online bot of a desired
    # class. ORDER BY level so the smoke test gets a stable choice
    # (usually the lowest-level one — easiest to reshape into a bracket).
    "character_online_by_class": QueryTemplate(
        name="character_online_by_class",
        db="acore_characters",
        sql=(
            "SELECT guid, name, race, class, level, online "
            "FROM characters "
            "WHERE class=%s AND online=1 "
            "ORDER BY level "
            "LIMIT 20"
        ),
        params=["class_id"],
    ),
    # Raw game_event rows for the Rust GES shadow-verify slice.
    # Schema verified 2026-05-30 via DESCRIBE acore_world.game_event:
    #   eventEntry, start_time, end_time, occurence (AC misspelling), length,
    #   holiday, holidayStage, description, world_event, announce.
    # Columns state / nextstart do NOT exist in this fork — they are C++
    # runtime-only values produced by GameEventMgr, not stored in the DB.
    # UNIX_TIMESTAMP converts the timestamp columns to integers so the Rust
    # deserializer can treat them as u64 epoch-seconds without tzinfo drift.
    # Parameter-free: always returns all 181 rows (152 periodic + 29 holiday).
    "game_event_all": QueryTemplate(
        name="game_event_all",
        db="acore_world",
        sql=(
            "SELECT eventEntry, "
            "       UNIX_TIMESTAMP(start_time) AS start_time, "
            "       UNIX_TIMESTAMP(end_time) AS end_time, "
            "       occurence, length, holiday, holidayStage, "
            "       description, world_event, announce "
            "FROM game_event "
            "ORDER BY eventEntry"
        ),
        params=[],
    ),
}


class DBClient:
    """aiomysql connection pool wrapped with the allowlist."""

    def __init__(self, host: str, port: int, user: str, password: str) -> None:
        self._host = host
        self._port = port
        self._user = user
        self._password = password
        self._pool: aiomysql.Pool | None = None

    async def _ensure_pool(self) -> aiomysql.Pool:
        if self._pool is None:
            self._pool = await aiomysql.create_pool(
                host=self._host, port=self._port,
                user=self._user, password=self._password,
                autocommit=True, minsize=1, maxsize=4,
            )
        return self._pool

    async def query(self, template_name: str, params: dict[str, Any]) -> list[dict]:
        if template_name not in V1_TEMPLATES:
            raise UnknownTemplate(template_name)
        tpl = V1_TEMPLATES[template_name]

        for p in tpl.params:
            if p not in params:
                raise BadParams(f"missing param '{p}'")
        param_tuple = tuple(params[p] for p in tpl.params)

        pool = await self._ensure_pool()
        async with pool.acquire() as conn:
            async with conn.cursor(aiomysql.DictCursor) as cur:
                await cur.execute(f"USE {tpl.db}")
                await cur.execute(tpl.sql, param_tuple)
                return list(await cur.fetchall())

    async def close(self) -> None:
        if self._pool is not None:
            self._pool.close()
            await self._pool.wait_closed()
