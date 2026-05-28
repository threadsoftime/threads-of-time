# SPDX-License-Identifier: GPL-2.0-or-later
"""Integration: SubsetGate driving StateStore via mock snapshot + enroll/release lambdas.

Tests verify Phase A behavior:
  - Cold start: 30 bots near a player → closest 10 enrolled.
  - Player logout: 10 enrolled bots, players leave → all released after 2 cycles
    (hysteresis_out_ticks=2; hysteresis_out_ticks+1 >= 2 triggers release on cycle 2).

Spec: docs/superpowers/specs/2026-05-27-threads-of-time-1.0.0-subset-gating-design.md §9
"""
from __future__ import annotations

import pytest

from brain_sidecar.subset_gate import (
    BotSnapshot,
    PlayerSnapshot,
    SubsetGate,
    SubsetGateConfig,
    WorldSnapshot,
)

from .conftest import _fake_state_store, _make_personality


# ---------------------------------------------------------------------------
# Test 1: cold-start enrolls the N closest bots
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_cold_start_enrolls_n_bots():
    """Cold start with 30 bots near Alice should enroll the closest 10.

    Bots are placed along the X-axis at x=0..29; Alice is at x=0.
    Distance equals bot index, so bots 100-109 (index 0-9) are closest.
    With hysteresis_in_ticks=1, a single _recompute_and_apply call is
    sufficient to enroll (in_ticks 0+1 >= 1).
    """
    state_store = _fake_state_store()

    snapshot = WorldSnapshot(
        players=(
            PlayerSnapshot(
                player_guid=1, name="Alice", map_id=0,
                x=0.0, y=0.0, z=0.0, level=10,
            ),
        ),
        bots=tuple(
            BotSnapshot(
                bot_guid=100 + i, name=f"b{i}", map_id=0,
                x=float(i), y=0.0, z=0.0, level=10,
            )
            for i in range(30)
        ),
    )

    enrolled: set[int] = set()
    released: set[int] = set()

    async def fetcher() -> WorldSnapshot:
        return snapshot

    async def enroll_fn(g: int) -> None:
        enrolled.add(g)
        # Mirror into state_store so _apply's post-apply list_active() is accurate.
        try:
            state_store.enroll(
                bot_guid=g, enrolled_at_ms=0, personality_seed=_make_personality()
            )
        except ValueError:
            pass  # already enrolled

    async def release_fn(g: int) -> None:
        released.add(g)
        state_store.set_status(g, "released")

    gate = SubsetGate(
        state_store=state_store,
        snapshot_fetcher=fetcher,
        enroll_fn=enroll_fn,
        release_fn=release_fn,
        config=SubsetGateConfig(
            living_bot_count=10,
            recompute_interval_s=60.0,
            hysteresis_out_ticks=2,
            hysteresis_in_ticks=1,
            enroll_backoff_s=300.0,
            enabled=True,
            phase_b_enabled=False,
        ),
    )

    await gate._recompute_and_apply()

    # The 10 closest bots are guids 100-109 (distance 0-9).
    assert enrolled == set(range(100, 110)), f"enrolled={enrolled}"
    assert released == set()


# ---------------------------------------------------------------------------
# Test 2: player logout → Phase A releases all bots after 2 hysteresis cycles
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_player_logs_out_releases_all_phase_a():
    """All players gone → after 2 cycles all bots released (Phase A).

    hysteresis_out_ticks=2: first cycle bumps out_ticks to 1 (< 2, no release);
    second cycle bumps out_ticks to 2, threshold met → release all 10.
    phase_b_enabled=False: no warm-cache; target = sticky only (empty) when no players.
    """
    state_store = _fake_state_store()

    # Pre-enroll 10 bots (simulating a prior active session).
    for g in range(100, 110):
        state_store.enroll(
            bot_guid=g, enrolled_at_ms=0, personality_seed=_make_personality()
        )

    # Two consecutive empty snapshots.
    snapshots = [
        WorldSnapshot(players=(), bots=()),
        WorldSnapshot(players=(), bots=()),
    ]
    idx = {"i": 0}

    async def fetcher() -> WorldSnapshot:
        s = snapshots[idx["i"]]
        idx["i"] = min(idx["i"] + 1, len(snapshots) - 1)
        return s

    released: set[int] = set()

    async def release_fn(g: int) -> None:
        released.add(g)
        state_store.set_status(g, "released")

    async def enroll_fn(g: int) -> None:
        pass  # no enrolls expected in this scenario

    gate = SubsetGate(
        state_store=state_store,
        snapshot_fetcher=fetcher,
        enroll_fn=enroll_fn,
        release_fn=release_fn,
        config=SubsetGateConfig(
            living_bot_count=10,
            recompute_interval_s=60.0,
            hysteresis_out_ticks=2,
            hysteresis_in_ticks=1,
            enroll_backoff_s=300.0,
            enabled=True,
            phase_b_enabled=False,
        ),
    )

    # Cycle 1 — out_ticks bumped to 1; threshold (1+1 >= 2) NOT met → no releases.
    await gate._recompute_and_apply()
    assert released == set(), f"Expected no releases after cycle 1, got {released}"

    # Cycle 2 — out_ticks bumped to 2; threshold (2+1 >= 2) → all released.
    await gate._recompute_and_apply()
    assert released == set(range(100, 110)), f"Expected all 10 released, got {released}"
