"""SubsetGate — periodic recompute of the brain's living-bot set.

Spec: docs/superpowers/specs/2026-05-27-threads-of-time-1.0.0-subset-gating-design.md
"""
# SPDX-License-Identifier: GPL-2.0-or-later
from __future__ import annotations

import asyncio
import logging
import math
import time
from dataclasses import dataclass, field
from typing import Awaitable, Callable, Optional


log = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Frozen dataclasses (pure-function surface)
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class PlayerSnapshot:
    player_guid: int
    name: str
    map_id: int
    x: float
    y: float
    z: float
    level: int
    party_guid: Optional[int] = None
    raid_guid: Optional[int] = None
    in_pvp_combat: bool = False


@dataclass(frozen=True)
class BotSnapshot:
    bot_guid: int
    name: str
    map_id: int
    x: float
    y: float
    z: float
    level: int
    party_guid: Optional[int] = None
    raid_guid: Optional[int] = None
    master_guid: Optional[int] = None
    in_pvp_combat: bool = False


@dataclass(frozen=True)
class WorldSnapshot:
    players: tuple[PlayerSnapshot, ...]
    bots: tuple[BotSnapshot, ...]


@dataclass(frozen=True)
class SubsetDecision:
    target_living_set: frozenset[int]
    sticky_pinned: frozenset[int]
    proximity_picks: frozenset[int]
    to_enroll: frozenset[int]
    to_release: frozenset[int]
    to_full: frozenset[int]
    to_reduced: frozenset[int]
    skipped_due_to_backoff: frozenset[int]


@dataclass(frozen=True)
class SubsetGateConfig:
    living_bot_count: int
    recompute_interval_s: float
    hysteresis_out_ticks: int
    hysteresis_in_ticks: int
    enroll_backoff_s: float
    enabled: bool
    phase_b_enabled: bool = False  # warm-cache + REDUCED tier


# ---------------------------------------------------------------------------
# Pure functions
# ---------------------------------------------------------------------------

def _euclid(a: PlayerSnapshot | BotSnapshot, b: PlayerSnapshot | BotSnapshot) -> float:
    return math.sqrt((a.x - b.x) ** 2 + (a.y - b.y) ** 2 + (a.z - b.z) ** 2)


def recompute_once(
    *,
    snapshot: WorldSnapshot,
    currently_enrolled: dict[int, int],   # bot_guid -> last_seen_ms (or 0)
    currently_pinned: frozenset[int],
    backoff_set: frozenset[int],
    hysteresis: dict[int, tuple[int, int]],   # bot_guid -> (in_ticks, out_ticks) — pre-bump
    config: SubsetGateConfig,
) -> SubsetDecision:
    """Pure function. No I/O. Computes the next SubsetDecision."""
    if not snapshot.players:
        # No players online.
        # Phase A: target = sticky only (no warm cache).
        # Phase B: target = sticky + currently_enrolled (warm-cache).
        target_proximity: frozenset[int] = frozenset()
        sticky: frozenset[int] = frozenset(currently_pinned)
        if config.phase_b_enabled:
            target = sticky | frozenset(currently_enrolled.keys())
        else:
            target = sticky
        return _reconcile(
            snapshot=snapshot,
            target=target,
            target_proximity=target_proximity,
            sticky=sticky,
            currently_enrolled=currently_enrolled,
            currently_pinned=currently_pinned,
            backoff_set=backoff_set,
            hysteresis=hysteresis,
            config=config,
            proximity_picks=frozenset(),
        )

    # Players present — compute sticky overrides + proximity target.
    sticky: set[int] = set(currently_pinned)
    for bot in snapshot.bots:
        for p in snapshot.players:
            if (
                (p.party_guid is not None and p.party_guid == bot.party_guid)
                or (p.raid_guid is not None and p.raid_guid == bot.raid_guid)
                or (bot.in_pvp_combat and bot.map_id == p.map_id)
            ):
                sticky.add(bot.bot_guid)
                break

    # Proximity ranking — same-map bots only, excluding sticky + backoff.
    eligible_bots = [
        b for b in snapshot.bots
        if any(p.map_id == b.map_id for p in snapshot.players)
        and b.bot_guid not in sticky
        and b.bot_guid not in backoff_set
    ]

    def _score(bot: BotSnapshot) -> float:
        same_map = [p for p in snapshot.players if p.map_id == bot.map_id]
        return min(_euclid(bot, p) for p in same_map) if same_map else math.inf

    eligible_bots.sort(key=lambda b: (_score(b), b.bot_guid))
    slots_remaining = max(0, config.living_bot_count - len(sticky))
    proximity_picks = frozenset(b.bot_guid for b in eligible_bots[:slots_remaining])
    target_proximity = frozenset(sticky) | proximity_picks

    # Phase B: warm-cache extends with currently-enrolled bots until capacity.
    if config.phase_b_enabled:
        warm_candidates = sorted(
            (g for g in currently_enrolled if g not in target_proximity),
            key=lambda g: currently_enrolled[g],
            reverse=True,
        )
        warm_slots = max(0, config.living_bot_count - len(target_proximity))
        target = target_proximity | frozenset(warm_candidates[:warm_slots])
    else:
        target = target_proximity

    return _reconcile(
        snapshot=snapshot,
        target=target,
        target_proximity=target_proximity,
        sticky=frozenset(sticky),
        currently_enrolled=currently_enrolled,
        currently_pinned=currently_pinned,
        backoff_set=backoff_set,
        hysteresis=hysteresis,
        config=config,
        proximity_picks=proximity_picks,
    )


def _reconcile(
    *,
    snapshot: WorldSnapshot,
    target: frozenset[int],
    target_proximity: frozenset[int],
    sticky: frozenset[int],
    currently_enrolled: dict[int, int],
    currently_pinned: frozenset[int],
    backoff_set: frozenset[int],
    hysteresis: dict[int, tuple[int, int]],
    config: SubsetGateConfig,
    proximity_picks: frozenset[int],
) -> SubsetDecision:
    to_enroll: set[int] = set()
    to_release: set[int] = set()

    # Determine releases: currently enrolled bots not in target + not pinned.
    for bot_guid in currently_enrolled:
        if bot_guid in target or bot_guid in currently_pinned:
            continue
        in_ticks, out_ticks = hysteresis.get(bot_guid, (0, 0))
        # Hypothetical post-bump out_ticks; if threshold met and not sticky, release.
        if out_ticks + 1 >= config.hysteresis_out_ticks and bot_guid not in sticky:
            to_release.add(bot_guid)

    # Determine enrolls: bots in target_proximity that are not yet enrolled.
    # Sticky bots bypass hysteresis; others must meet hysteresis_in_ticks.
    for bot_guid in target_proximity:
        if bot_guid in currently_enrolled:
            continue
        if bot_guid in backoff_set:
            continue
        if bot_guid in sticky:
            # Sticky bypass: enroll immediately, no hysteresis gate.
            to_enroll.add(bot_guid)
            continue
        in_ticks, _ = hysteresis.get(bot_guid, (0, 0))
        if in_ticks + 1 >= config.hysteresis_in_ticks:
            to_enroll.add(bot_guid)

    # Phase B tier assignment.
    to_full: set[int] = set()
    to_reduced: set[int] = set()
    if config.phase_b_enabled:
        kept = (set(currently_enrolled) - to_release) | to_enroll
        bot_by_guid = {b.bot_guid: b for b in snapshot.bots}
        for guid in kept:
            bot = bot_by_guid.get(guid)
            # A bot absent from the snapshot (empty world / no players) has no
            # co-located player → always REDUCED.  A bot present in the snapshot
            # is FULL iff at least one player shares its map_id; otherwise REDUCED.
            if bot is not None and any(p.map_id == bot.map_id for p in snapshot.players):
                to_full.add(guid)
            else:
                to_reduced.add(guid)

    # skipped_due_to_backoff: backoff bots that are in the snapshot.
    skipped = backoff_set & frozenset(b.bot_guid for b in snapshot.bots)

    return SubsetDecision(
        target_living_set=target,
        sticky_pinned=sticky,
        proximity_picks=proximity_picks,
        to_enroll=frozenset(to_enroll),
        to_release=frozenset(to_release),
        to_full=frozenset(to_full),
        to_reduced=frozenset(to_reduced),
        skipped_due_to_backoff=skipped,
    )


# ---------------------------------------------------------------------------
# SubsetGate class — asyncio task wrapper
# ---------------------------------------------------------------------------

SnapshotFetcher = Callable[[], Awaitable[WorldSnapshot]]
EnrollFn = Callable[[int], Awaitable[None]]
ReleaseFn = Callable[[int], Awaitable[None]]


@dataclass
class SubsetGate:
    state_store: object  # StateStore (avoid import cycle)
    snapshot_fetcher: SnapshotFetcher
    enroll_fn: EnrollFn
    release_fn: ReleaseFn
    config: SubsetGateConfig
    backoff_state: dict[int, float] = field(default_factory=dict)  # bot_guid -> deadline_ts
    _task: Optional[asyncio.Task] = field(default=None)

    async def run(self) -> None:
        if not self.config.enabled:
            log.info("SubsetGate disabled via config — exiting run() immediately")
            return
        log.info("SubsetGate.run() started; interval=%.0fs", self.config.recompute_interval_s)
        while True:
            try:
                await self._recompute_and_apply()
            except asyncio.CancelledError:
                raise
            except Exception:
                log.exception("SubsetGate recompute cycle failed; will retry next tick")
            try:
                await asyncio.sleep(self.config.recompute_interval_s)
            except asyncio.CancelledError:
                raise

    async def _recompute_and_apply(self) -> SubsetDecision:
        snapshot = await self.snapshot_fetcher()
        currently_enrolled = {
            row.bot_guid: row.last_seen or 0
            for row in self.state_store.list_active()
        }
        currently_pinned = frozenset(self.state_store.list_pinned())
        backoff_set = self._expired_backoff_set()
        hysteresis = {
            g: self.state_store.get_hysteresis(g) for g in currently_enrolled
        }
        decision = recompute_once(
            snapshot=snapshot,
            currently_enrolled=currently_enrolled,
            currently_pinned=currently_pinned,
            backoff_set=backoff_set,
            hysteresis=hysteresis,
            config=self.config,
        )
        await self._apply(decision)
        return decision

    def _expired_backoff_set(self) -> frozenset[int]:
        now = time.time()
        live = {g: dl for g, dl in self.backoff_state.items() if dl > now}
        self.backoff_state = live
        return frozenset(live.keys())

    async def _apply(self, decision: SubsetDecision) -> None:
        for bot_guid in decision.to_release:
            try:
                await self.release_fn(bot_guid)
            except Exception:
                log.exception("subset gate release failed bot_guid=%d", bot_guid)

        for bot_guid in decision.to_enroll:
            try:
                await self.enroll_fn(bot_guid)
            except Exception:
                log.exception("subset gate enroll failed bot_guid=%d", bot_guid)
                self.backoff_state[bot_guid] = time.time() + self.config.enroll_backoff_s

        for bot_guid in decision.to_full:
            self.state_store.set_tier(bot_guid, "full")
        for bot_guid in decision.to_reduced:
            self.state_store.set_tier(bot_guid, "reduced")

        # Bump hysteresis counters for all currently enrolled (post-apply).
        currently_enrolled = frozenset(
            row.bot_guid for row in self.state_store.list_active()
        )
        target = decision.target_living_set
        for bot_guid in currently_enrolled:
            self.state_store.bump_hysteresis(bot_guid, in_range=(bot_guid in target))
