"""C6: SQLite state store (living_bots + decisions_recent ring buffer)."""
from __future__ import annotations

import json
import sqlite3
import threading
from collections.abc import Iterator
from dataclasses import dataclass
from importlib import resources
from pathlib import Path

from brain_sidecar.models import Decision, PersonalityCard

_RING_DEPTH = 20


@dataclass
class LivingBotRow:
    bot_guid: int
    enrolled_at: int
    last_seen: int | None
    status: str
    personality_seed: PersonalityCard


class StateStore:
    """Thin sync wrapper over a sqlite3 connection. Called from asyncio code via run_in_executor."""

    def __init__(self, db_path: str) -> None:
        self._db_path = db_path
        Path(db_path).parent.mkdir(parents=True, exist_ok=True)
        self._conn = sqlite3.connect(db_path, check_same_thread=False, isolation_level=None)
        self._conn.execute("PRAGMA journal_mode=WAL")
        self._conn.execute("PRAGMA foreign_keys=ON")
        # Serialise concurrent writes from multiple bot-loop executor threads.
        # sqlite3 with check_same_thread=False allows multi-thread access but
        # non-atomic SELECT+INSERT sequences (e.g. append_decision) still race.
        self._write_lock = threading.Lock()

    def migrate(self) -> None:
        """Apply migrations in sorted order, skipping already-applied ones.

        Each migration file is named ``NNNN_*.sql`` where NNNN is an integer
        version (e.g. ``0001_living_bots.sql`` → version 1).  The
        ``schema_version`` table tracks the highest version applied.  Migrations
        with version <= current max are skipped, making the method idempotent.

        Migration 0001 bootstraps the schema_version table itself, so it is
        always executed via executescript (safe because it uses IF NOT EXISTS
        everywhere).  Subsequent migrations are executed only once.
        """
        migrations_dir = Path(__file__).resolve().parent.parent / "migrations"
        for sql_file in sorted(migrations_dir.glob("*.sql")):
            # Extract the leading integer from the filename (e.g. "0002" → 2).
            stem = sql_file.stem  # e.g. "0002_add_last_event_id"
            try:
                file_version = int(stem.split("_")[0])
            except (ValueError, IndexError):
                # Non-standard filename; run unconditionally.
                file_version = None

            if file_version is not None and file_version > 1:
                # Check whether this migration has already been applied.
                try:
                    row = self._conn.execute(
                        "SELECT MAX(version) FROM schema_version"
                    ).fetchone()
                    current_version = int(row[0]) if row and row[0] is not None else 0
                except Exception:
                    current_version = 0
                if current_version >= file_version:
                    continue  # already applied

            self._conn.executescript(sql_file.read_text(encoding="utf-8"))

    def close(self) -> None:
        self._conn.close()

    def enroll(self, *, bot_guid: int, enrolled_at_ms: int, personality_seed: PersonalityCard) -> None:
        existing = self._conn.execute(
            "SELECT bot_guid FROM living_bots WHERE bot_guid = ? AND status != 'released'",
            (bot_guid,),
        ).fetchone()
        if existing is not None:
            raise ValueError(f"bot_guid {bot_guid} already enrolled")
        seed_json = personality_seed.model_dump_json(by_alias=True)
        self._conn.execute(
            "INSERT OR REPLACE INTO living_bots "
            "(bot_guid, enrolled_at, last_seen, status, personality_seed_json) "
            "VALUES (?, ?, ?, 'active', ?)",
            (bot_guid, enrolled_at_ms, None, seed_json),
        )

    def set_status(self, bot_guid: int, status: str) -> None:
        self._conn.execute(
            "UPDATE living_bots SET status = ? WHERE bot_guid = ?",
            (status, bot_guid),
        )

    def get_bot(self, bot_guid: int) -> LivingBotRow | None:
        row = self._conn.execute(
            "SELECT bot_guid, enrolled_at, last_seen, status, personality_seed_json "
            "FROM living_bots WHERE bot_guid = ?",
            (bot_guid,),
        ).fetchone()
        if row is None:
            return None
        return LivingBotRow(
            bot_guid=row[0],
            enrolled_at=row[1],
            last_seen=row[2],
            status=row[3],
            personality_seed=PersonalityCard.model_validate_json(row[4]),
        )

    def list_active(self) -> Iterator[LivingBotRow]:
        cur = self._conn.execute(
            "SELECT bot_guid, enrolled_at, last_seen, status, personality_seed_json "
            "FROM living_bots WHERE status = 'active'"
        )
        for row in cur:
            yield LivingBotRow(
                bot_guid=row[0],
                enrolled_at=row[1],
                last_seen=row[2],
                status=row[3],
                personality_seed=PersonalityCard.model_validate_json(row[4]),
            )

    def append_decision(self, *, bot_guid: int, ts_ms: int, decision: Decision) -> None:
        # seq is monotonically increasing per bot. Ring-buffer semantics enforced by
        # pruning all rows older than the most recent _RING_DEPTH entries.
        # Lock protects the SELECT+INSERT against concurrent executor-thread calls when
        # multiple bot loops share the same StateStore instance.
        with self._write_lock:
            row = self._conn.execute(
                "SELECT COALESCE(MAX(seq), -1) FROM decisions_recent WHERE bot_guid = ?",
                (bot_guid,),
            ).fetchone()
            next_seq = int(row[0]) + 1
            self._conn.execute(
                "INSERT INTO decisions_recent (bot_guid, seq, ts_ms, decision_json) "
                "VALUES (?, ?, ?, ?)",
                (bot_guid, next_seq, ts_ms, decision.model_dump_json()),
            )
            # Prune anything older than the newest _RING_DEPTH rows.
            self._conn.execute(
                "DELETE FROM decisions_recent WHERE bot_guid = ? AND seq <= ?",
                (bot_guid, next_seq - _RING_DEPTH),
            )

    def read_last_event_id(self, bot_guid: int) -> int:
        """Return the last SSE row_id persisted for this bot (0 if unknown/unenrolled)."""
        row = self._conn.execute(
            "SELECT last_event_id FROM living_bots WHERE bot_guid = ?",
            (bot_guid,),
        ).fetchone()
        return int(row[0]) if row is not None else 0

    def write_last_event_id(self, bot_guid: int, event_id: int) -> None:
        """Monotonic update — never regress to a lower value."""
        self._conn.execute(
            "UPDATE living_bots SET last_event_id = ? "
            "WHERE bot_guid = ? AND last_event_id < ?",
            (event_id, bot_guid, event_id),
        )
        self._conn.commit()

    # ------------------------------------------------------------------
    # Subset-gating helpers (Plan 3, Tasks 9-11)
    # ------------------------------------------------------------------

    def set_tier(self, bot_guid: int, tier: str) -> None:
        if tier not in ("full", "reduced"):
            raise ValueError(f"invalid tier {tier!r}")
        with self._write_lock:
            self._conn.execute(
                "UPDATE living_bots SET tier = ? WHERE bot_guid = ?",
                (tier, bot_guid),
            )

    def get_tier(self, bot_guid: int) -> str:
        row = self._conn.execute(
            "SELECT tier FROM living_bots WHERE bot_guid = ?",
            (bot_guid,),
        ).fetchone()
        if row is None or row[0] is None:
            return "full"
        return row[0]

    def get_hysteresis(self, bot_guid: int) -> tuple[int, int]:
        row = self._conn.execute(
            "SELECT in_range_ticks, out_of_range_ticks FROM living_bots WHERE bot_guid = ?",
            (bot_guid,),
        ).fetchone()
        if row is None:
            return (0, 0)
        return (row[0] or 0, row[1] or 0)

    def bump_hysteresis(self, bot_guid: int, *, in_range: bool) -> tuple[int, int]:
        with self._write_lock:
            row = self._conn.execute(
                "SELECT in_range_ticks, out_of_range_ticks FROM living_bots WHERE bot_guid = ?",
                (bot_guid,),
            ).fetchone()
            if row is None:
                return (0, 0)
            in_ticks = row[0] or 0
            out_ticks = row[1] or 0
            if in_range:
                in_ticks += 1
                out_ticks = 0
            else:
                in_ticks = 0
                out_ticks += 1
            self._conn.execute(
                "UPDATE living_bots SET in_range_ticks = ?, out_of_range_ticks = ? WHERE bot_guid = ?",
                (in_ticks, out_ticks, bot_guid),
            )
            return (in_ticks, out_ticks)

    def reactivate(self, bot_guid: int) -> None:
        """Flip a bot back to status='active' and reset hysteresis counters.

        Used by SubsetGate when re-enrolling a bot whose status is 'released'
        (e.g., warm-cache eviction). Atomic; no-op if the bot doesn't exist.
        """
        with self._write_lock:
            self._conn.execute(
                "UPDATE living_bots "
                "SET status='active', in_range_ticks=0, out_of_range_ticks=0 "
                "WHERE bot_guid = ?",
                (bot_guid,),
            )

    def set_pin(self, bot_guid: int, pinned: bool) -> None:
        with self._write_lock:
            self._conn.execute(
                "UPDATE living_bots SET pinned = ? WHERE bot_guid = ?",
                (1 if pinned else 0, bot_guid),
            )

    def list_pinned(self) -> list[int]:
        rows = self._conn.execute(
            "SELECT bot_guid FROM living_bots WHERE pinned = 1"
        ).fetchall()
        return [r[0] for r in rows]

    def decisions_recent(self, *, bot_guid: int, k: int) -> list[Decision]:
        rows = self._conn.execute(
            "SELECT decision_json FROM decisions_recent "
            "WHERE bot_guid = ? ORDER BY ts_ms DESC LIMIT ?",
            (bot_guid, k),
        ).fetchall()
        return [Decision.model_validate_json(r[0]) for r in rows]
