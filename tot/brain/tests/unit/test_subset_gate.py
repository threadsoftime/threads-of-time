# SPDX-License-Identifier: GPL-2.0-or-later
"""Unit tests for brain_sidecar.subset_gate — Tasks 13-19.

Spec: docs/superpowers/specs/2026-05-27-threads-of-time-1.0.0-subset-gating-design.md
"""
from __future__ import annotations

import asyncio
from unittest.mock import AsyncMock, MagicMock

from brain_sidecar.subset_gate import (
    BotSnapshot,
    PlayerSnapshot,
    SubsetDecision,
    SubsetGate,
    SubsetGateConfig,
    WorldSnapshot,
    recompute_once,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _cfg(**overrides) -> SubsetGateConfig:
    base = dict(
        living_bot_count=10,
        recompute_interval_s=60.0,
        hysteresis_out_ticks=2,
        hysteresis_in_ticks=1,
        enroll_backoff_s=300.0,
        enabled=True,
        phase_b_enabled=False,
    )
    base.update(overrides)
    return SubsetGateConfig(**base)


# ---------------------------------------------------------------------------
# Task 13 — construction smoke test
# ---------------------------------------------------------------------------

def test_world_snapshot_construction():
    p = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                       x=0.0, y=0.0, z=0.0, level=10)
    b = BotSnapshot(bot_guid=100, name="Borg", map_id=0,
                    x=10.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(p,), bots=(b,))
    assert snap.players[0].name == "Alice"
    assert snap.bots[0].master_guid is None


# ---------------------------------------------------------------------------
# Task 14 — empty world / no-players paths
# ---------------------------------------------------------------------------

def test_empty_world_yields_empty_target():
    snap = WorldSnapshot(players=(), bots=())
    decision = recompute_once(
        snapshot=snap,
        currently_enrolled={},
        currently_pinned=frozenset(),
        backoff_set=frozenset(),
        hysteresis={},
        config=_cfg(),
    )
    assert decision.target_living_set == frozenset()
    assert decision.to_enroll == frozenset()
    assert decision.to_release == frozenset()


def test_bots_present_no_players_yields_empty_target_phase_a():
    bots = tuple(
        BotSnapshot(bot_guid=i, name=f"b{i}", map_id=0, x=float(i),
                    y=0.0, z=0.0, level=10)
        for i in range(20)
    )
    snap = WorldSnapshot(players=(), bots=bots)
    decision = recompute_once(
        snapshot=snap,
        currently_enrolled={},
        currently_pinned=frozenset(),
        backoff_set=frozenset(),
        hysteresis={},
        config=_cfg(),
    )
    assert decision.target_living_set == frozenset()


# ---------------------------------------------------------------------------
# Task 15 — proximity ranking
# ---------------------------------------------------------------------------

def test_single_player_closest_n_wins():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bots = tuple(
        BotSnapshot(bot_guid=100 + i, name=f"b{i}", map_id=0,
                    x=float(i), y=0.0, z=0.0, level=10)
        for i in range(20)  # bots at x=0..19 on map 0
    )
    snap = WorldSnapshot(players=(alice,), bots=bots)
    decision = recompute_once(
        snapshot=snap,
        currently_enrolled={},
        currently_pinned=frozenset(),
        backoff_set=frozenset(),
        hysteresis={},
        config=_cfg(living_bot_count=5, hysteresis_in_ticks=1),
    )
    # Closest 5 bots are b0..b4 (bot_guids 100..104)
    assert decision.to_enroll == frozenset({100, 101, 102, 103, 104})


def test_different_map_excluded_from_proximity():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    near_other_map = BotSnapshot(bot_guid=100, name="far", map_id=1,
                                  x=0.0, y=0.0, z=0.0, level=10)
    far_same_map = BotSnapshot(bot_guid=200, name="near", map_id=0,
                                x=999.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(alice,), bots=(near_other_map, far_same_map))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5),
    )
    assert decision.to_enroll == frozenset({200})


# ---------------------------------------------------------------------------
# Task 16 — sticky overrides
# ---------------------------------------------------------------------------

def test_sticky_party_overrides_proximity():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10, party_guid=42)
    far_party_bot = BotSnapshot(bot_guid=999, name="far_party", map_id=0,
                                 x=10000.0, y=0.0, z=0.0, level=10, party_guid=42)
    near_other_bots = tuple(
        BotSnapshot(bot_guid=100 + i, name=f"b{i}", map_id=0,
                    x=float(i), y=0.0, z=0.0, level=10)
        for i in range(10)
    )
    snap = WorldSnapshot(players=(alice,), bots=near_other_bots + (far_party_bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5),
    )
    # 999 is sticky (party); 4 closest non-sticky fill remaining slots.
    assert 999 in decision.to_enroll
    assert decision.sticky_pinned == frozenset({999})


def test_sticky_raid_overrides_proximity():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10, raid_guid=77)
    raid_bot = BotSnapshot(bot_guid=500, name="r", map_id=0,
                            x=10000.0, y=0.0, z=0.0, level=10, raid_guid=77)
    snap = WorldSnapshot(players=(alice,), bots=(raid_bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5),
    )
    assert 500 in decision.sticky_pinned
    assert 500 in decision.to_enroll


def test_sticky_pvp_combat_requires_same_map():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    pvp_bot_same_map = BotSnapshot(bot_guid=300, name="pvp_near", map_id=0,
                                    x=5000.0, y=0.0, z=0.0, level=10,
                                    in_pvp_combat=True)
    pvp_bot_other_map = BotSnapshot(bot_guid=400, name="pvp_far", map_id=99,
                                     x=0.0, y=0.0, z=0.0, level=10,
                                     in_pvp_combat=True)
    snap = WorldSnapshot(players=(alice,), bots=(pvp_bot_same_map, pvp_bot_other_map))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5),
    )
    assert 300 in decision.sticky_pinned
    assert 400 not in decision.sticky_pinned


# ---------------------------------------------------------------------------
# Task 17 — hysteresis state machine + pin bypass
# ---------------------------------------------------------------------------

def test_enroll_requires_hysteresis_in_ticks():
    # Bot has been in range 0 ticks; hysteresis_in_ticks=2 — should NOT enroll yet.
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bot = BotSnapshot(bot_guid=100, name="b", map_id=0,
                      x=1.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(alice,), bots=(bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5, hysteresis_in_ticks=2),
    )
    # in_ticks=0 -> post-bump = 1, which is < 2, so NOT enrolled this cycle.
    assert 100 not in decision.to_enroll


def test_release_requires_hysteresis_out_ticks():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bot = BotSnapshot(bot_guid=100, name="b", map_id=1,  # different map
                      x=0.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(alice,), bots=(bot,))
    # Bot currently enrolled, hysteresis (0, 0); needs out_ticks >= 2 to release.
    decision1 = recompute_once(
        snapshot=snap, currently_enrolled={100: 0}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={100: (0, 0)},
        config=_cfg(living_bot_count=5, hysteresis_out_ticks=2),
    )
    # out_ticks would bump to 1, < 2 — NOT released yet.
    assert 100 not in decision1.to_release

    decision2 = recompute_once(
        snapshot=snap, currently_enrolled={100: 0}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={100: (0, 1)},
        config=_cfg(living_bot_count=5, hysteresis_out_ticks=2),
    )
    # out_ticks bumps to 2 — release.
    assert 100 in decision2.to_release


def test_sticky_bypasses_in_hysteresis():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10, party_guid=42)
    bot = BotSnapshot(bot_guid=100, name="b", map_id=0,
                      x=1.0, y=0.0, z=0.0, level=10, party_guid=42)
    snap = WorldSnapshot(players=(alice,), bots=(bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5, hysteresis_in_ticks=99),
    )
    # Sticky enrollment ignores hysteresis_in_ticks.
    assert 100 in decision.to_enroll


def test_pin_skips_release_even_when_out_of_range():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bot = BotSnapshot(bot_guid=100, name="b", map_id=99,  # other map
                      x=0.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(alice,), bots=(bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={100: 0},
        currently_pinned=frozenset({100}),
        backoff_set=frozenset(),
        hysteresis={100: (0, 99)},  # massively out-of-range
        config=_cfg(living_bot_count=5, hysteresis_out_ticks=2),
    )
    assert 100 not in decision.to_release


# ---------------------------------------------------------------------------
# Task 18 — backoff + multi-player + undersized-population edges
# ---------------------------------------------------------------------------

def test_backoff_set_excludes_from_enroll():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bot = BotSnapshot(bot_guid=100, name="b", map_id=0,
                      x=1.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(alice,), bots=(bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={},
        currently_pinned=frozenset(),
        backoff_set=frozenset({100}),
        hysteresis={},
        config=_cfg(living_bot_count=5),
    )
    assert 100 not in decision.to_enroll
    assert 100 in decision.skipped_due_to_backoff


def test_two_players_two_maps_closest_n_globally():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bob = PlayerSnapshot(player_guid=2, name="Bob", map_id=1,
                         x=100.0, y=0.0, z=0.0, level=10)
    # 3 bots on Alice's map (dist 1, 2, 3) + 3 bots on Bob's map (dist 1, 2, 3)
    bots = (
        BotSnapshot(bot_guid=10, name="a1", map_id=0, x=1.0, y=0.0, z=0.0, level=10),
        BotSnapshot(bot_guid=11, name="a2", map_id=0, x=2.0, y=0.0, z=0.0, level=10),
        BotSnapshot(bot_guid=12, name="a3", map_id=0, x=3.0, y=0.0, z=0.0, level=10),
        BotSnapshot(bot_guid=20, name="b1", map_id=1, x=101.0, y=0.0, z=0.0, level=10),
        BotSnapshot(bot_guid=21, name="b2", map_id=1, x=102.0, y=0.0, z=0.0, level=10),
        BotSnapshot(bot_guid=22, name="b3", map_id=1, x=103.0, y=0.0, z=0.0, level=10),
    )
    snap = WorldSnapshot(players=(alice, bob), bots=bots)
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=4),
    )
    # Top 4 = a1 + b1 + a2 + b2 (distances 1, 1, 2, 2)
    assert decision.to_enroll == frozenset({10, 11, 20, 21})


def test_population_smaller_than_living_count_uses_all():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bots = tuple(
        BotSnapshot(bot_guid=100 + i, name=f"b{i}", map_id=0,
                    x=float(i), y=0.0, z=0.0, level=10)
        for i in range(3)
    )
    snap = WorldSnapshot(players=(alice,), bots=bots)
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=10),
    )
    assert decision.to_enroll == frozenset({100, 101, 102})


# ---------------------------------------------------------------------------
# Task 19 — SubsetGate class: _apply integration test
# ---------------------------------------------------------------------------

def test_apply_calls_enroll_release_and_set_tier():
    state_store = MagicMock()
    state_store.list_active.return_value = iter([
        MagicMock(bot_guid=g, last_seen=0) for g in (1, 2, 3)
    ])
    state_store.list_pinned.return_value = []
    state_store.get_hysteresis.return_value = (0, 0)

    enroll_fn = AsyncMock()
    release_fn = AsyncMock()

    gate = SubsetGate(
        state_store=state_store,
        snapshot_fetcher=AsyncMock(return_value=WorldSnapshot(players=(), bots=())),
        enroll_fn=enroll_fn,
        release_fn=release_fn,
        config=_cfg(),
    )

    decision = SubsetDecision(
        target_living_set=frozenset(),
        sticky_pinned=frozenset(),
        proximity_picks=frozenset(),
        to_enroll=frozenset({4}),
        to_release=frozenset({1}),
        to_full=frozenset({2, 4}),
        to_reduced=frozenset({3}),
        skipped_due_to_backoff=frozenset(),
    )
    asyncio.run(gate._apply(decision))

    release_fn.assert_awaited_once_with(1)
    enroll_fn.assert_awaited_once_with(4, None)
    set_tier_calls = state_store.set_tier.call_args_list
    assert any(c.args == (2, "full") for c in set_tier_calls)
    assert any(c.args == (3, "reduced") for c in set_tier_calls)
    assert any(c.args == (4, "full") for c in set_tier_calls)


def test_apply_passes_bot_snapshot_to_enroll_fn():
    """B4: enroll_fn receives the matching BotSnapshot so it can bootstrap missing bots."""
    state_store = MagicMock()
    state_store.list_active.return_value = iter([])
    state_store.list_pinned.return_value = []
    state_store.get_hysteresis.return_value = (0, 0)

    enroll_fn = AsyncMock()
    release_fn = AsyncMock()

    bot_5 = BotSnapshot(bot_guid=5, name="Eve", map_id=0,
                       x=1.0, y=2.0, z=3.0, level=12)
    bot_7 = BotSnapshot(bot_guid=7, name="Gabe", map_id=0,
                       x=4.0, y=5.0, z=6.0, level=14)
    snapshot = WorldSnapshot(players=(), bots=(bot_5, bot_7))

    gate = SubsetGate(
        state_store=state_store,
        snapshot_fetcher=AsyncMock(return_value=snapshot),
        enroll_fn=enroll_fn,
        release_fn=release_fn,
        config=_cfg(),
    )

    decision = SubsetDecision(
        target_living_set=frozenset({5, 7, 99}),
        sticky_pinned=frozenset(),
        proximity_picks=frozenset({5, 7, 99}),
        to_enroll=frozenset({5, 7, 99}),  # 99 is not in the snapshot
        to_release=frozenset(),
        to_full=frozenset(),
        to_reduced=frozenset(),
        skipped_due_to_backoff=frozenset(),
    )
    asyncio.run(gate._apply(decision, snapshot=snapshot))

    # Each enroll_fn call was made with the matching BotSnapshot (or None when absent).
    by_guid = {c.args[0]: c.args[1] for c in enroll_fn.await_args_list}
    assert by_guid[5] is bot_5
    assert by_guid[7] is bot_7
    assert by_guid[99] is None
