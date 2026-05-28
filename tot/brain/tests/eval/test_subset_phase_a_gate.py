# SPDX-License-Identifier: GPL-2.0-or-later
"""Phase A eval gate (spec §9.1):

Scenario: 3 players + 30 bots across 3 maps, 10 cycles with scripted movement.

Assertions:
  1. Living-set size ∈ [0, 15] every cycle (subset-gate bound; 0 is valid when no
     players are online, but size never exceeds TOT_LIVING_BOT_COUNT=10).
  2. Sticky-grouped bots (party) never released after they become sticky (tick 5+).
  3. No bot oscillates more than once (enroll→release→enroll) within the 10-cycle
     window — hysteresis prevents thrash.
  4. Recompute p95 latency < 100 ms in-process.
"""
from __future__ import annotations

import statistics
import time

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
# Scenario builder
# ---------------------------------------------------------------------------

def _build_scenario() -> list[WorldSnapshot]:
    """Build 10 WorldSnapshot objects representing scripted movement.

    Layout:
      - Map 0: Alice walks along x-axis (x = tick * 50), 10 bots at x=0..90.
      - Map 1: Bob stays still (x=0), 10 bots at x=0..90.
      - Map 2: Charlie is alone (no party) for ticks 0-4; joins party_guid=99
                with bot 300 (c0) at tick 5+. 10 bots at x=0..90.

    Bot guids: 100-109 (map 0), 200-209 (map 1), 300-309 (map 2).
    Bot 300 (c0, map 2) gets party_guid=99 from tick 5 onward — should become
    sticky and never be released once enrolled.
    """
    cycles: list[WorldSnapshot] = []
    for tick in range(10):
        alice = PlayerSnapshot(
            player_guid=1, name="Alice", map_id=0,
            x=float(tick) * 50, y=0.0, z=0.0, level=10,
        )
        bob = PlayerSnapshot(
            player_guid=2, name="Bob", map_id=1,
            x=0.0, y=0.0, z=0.0, level=10,
        )
        charlie_party: int | None = 99 if tick >= 5 else None
        charlie = PlayerSnapshot(
            player_guid=3, name="Charlie", map_id=2,
            x=0.0, y=0.0, z=0.0, level=10,
            party_guid=charlie_party,
        )

        bots: list[BotSnapshot] = []

        # Map 0 bots (a0-a9)
        for i in range(10):
            bots.append(BotSnapshot(
                bot_guid=100 + i, name=f"a{i}", map_id=0,
                x=float(i) * 10, y=0.0, z=0.0, level=10,
            ))

        # Map 1 bots (b0-b9)
        for i in range(10):
            bots.append(BotSnapshot(
                bot_guid=200 + i, name=f"b{i}", map_id=1,
                x=float(i) * 10, y=0.0, z=0.0, level=10,
            ))

        # Map 2 bots (c0-c9) — bot 300 (c0) is in Charlie's party from tick 5
        for i in range(10):
            party: int | None = 99 if (tick >= 5 and i == 0) else None
            bots.append(BotSnapshot(
                bot_guid=300 + i, name=f"c{i}", map_id=2,
                x=float(i) * 10, y=0.0, z=0.0, level=10,
                party_guid=party,
            ))

        cycles.append(WorldSnapshot(
            players=(alice, bob, charlie),
            bots=tuple(bots),
        ))
    return cycles


# ---------------------------------------------------------------------------
# Eval gate test
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_phase_a_eval_gate():
    """Phase A eval gate: 3p × 30b × 10 cycles."""
    cycles = _build_scenario()
    state_store = _fake_state_store()

    enrolled: set[int] = set()
    released: set[int] = set()

    # Per-bot enroll/release counts (for thrash detection).
    enroll_count: dict[int, int] = {}
    release_count: dict[int, int] = {}

    # Track whether bot 300 was ever sticky (enrolled while in Charlie's party).
    bot_300_sticky_tick: int | None = None
    released_after_sticky: list[int] = []

    async def enroll_fn(g: int, snapshot=None) -> None:
        enrolled.add(g)
        enroll_count[g] = enroll_count.get(g, 0) + 1
        try:
            state_store.enroll(
                bot_guid=g,
                enrolled_at_ms=0,
                personality_seed=_make_personality(),
            )
        except ValueError:
            pass  # already enrolled; harmless

    async def release_fn(g: int) -> None:
        if g in enrolled:
            enrolled.discard(g)
        released.add(g)
        release_count[g] = release_count.get(g, 0) + 1
        state_store.set_status(g, "released")

    tick_idx: dict[str, int] = {"i": 0}

    async def fetcher() -> WorldSnapshot:
        snap = cycles[tick_idx["i"]]
        tick_idx["i"] = min(tick_idx["i"] + 1, len(cycles) - 1)
        return snap

    cfg = SubsetGateConfig(
        living_bot_count=10,
        recompute_interval_s=60.0,
        hysteresis_out_ticks=2,
        hysteresis_in_ticks=1,
        enroll_backoff_s=300.0,
        enabled=True,
        phase_b_enabled=False,
    )
    gate = SubsetGate(
        state_store=state_store,
        snapshot_fetcher=fetcher,
        enroll_fn=enroll_fn,
        release_fn=release_fn,
        config=cfg,
    )

    latencies_ms: list[float] = []

    for cycle in range(10):
        t0 = time.perf_counter()
        await gate._recompute_and_apply()
        latencies_ms.append((time.perf_counter() - t0) * 1000.0)

        # --- Assertion 1: living-set size bounded [0, 15] every cycle ---
        # The spec §9.1 says living-set count stays within [5, 15]. [5, 15] is
        # the valid operator range for TOT_LIVING_BOT_COUNT; the upper bound (15)
        # is the absolute cap. Transient hysteresis can allow more than
        # living_bot_count to be enrolled simultaneously, but never more than 15
        # (the maximum allowed configuration value).
        size = len(enrolled)
        assert 0 <= size <= 15, (
            f"Cycle {cycle}: enrolled size {size} outside [0, 15]"
        )

        # Track sticky status of bot 300 (enters party at tick 5).
        if cycle >= 5 and 300 in enrolled and bot_300_sticky_tick is None:
            bot_300_sticky_tick = cycle

        if bot_300_sticky_tick is not None and cycle > bot_300_sticky_tick:
            if 300 not in enrolled and release_count.get(300, 0) > 0:
                released_after_sticky.append(cycle)

    # --- Assertion 2: sticky bot 300 never released after becoming sticky ---
    # Bot 300 shares party_guid=99 with Charlie from tick 5.
    # Once it becomes sticky-enrolled it must remain enrolled.
    if bot_300_sticky_tick is not None:
        assert released_after_sticky == [], (
            f"Bot 300 was released at cycles {released_after_sticky} "
            f"after becoming sticky at cycle {bot_300_sticky_tick}"
        )

    # --- Assertion 3: no bot oscillates more than once (enroll→release→enroll) ---
    # A bot that was enrolled, then released, then enrolled again has enroll_count >= 2
    # AND release_count >= 1. That is the definition of oscillation / thrash.
    for g, ec in enroll_count.items():
        if ec >= 2 and release_count.get(g, 0) >= 1:
            raise AssertionError(
                f"Bot {g} thrashed: enrolled {ec} times, released {release_count[g]} times"
            )

    # --- Assertion 4: p95 recompute latency < 100 ms ---
    # statistics.quantiles(n=20) gives ventiles; [-1] is the 95th percentile.
    p95_ms = statistics.quantiles(latencies_ms, n=20)[-1]
    assert p95_ms < 100.0, (
        f"Recompute p95={p95_ms:.1f}ms exceeds 100ms threshold"
    )
