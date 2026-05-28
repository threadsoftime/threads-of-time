# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for subset-gating env vars in Settings (Task 12)."""
from __future__ import annotations

import pytest

from brain_sidecar.settings import get_settings


def test_subset_gate_defaults(monkeypatch):
    for k in ("TOT_SUBSET_GATE_ENABLED", "TOT_LIVING_BOT_COUNT",
              "TOT_SUBSET_RECOMPUTE_INTERVAL_S",
              "TOT_SUBSET_HYSTERESIS_OUT_TICKS",
              "TOT_SUBSET_HYSTERESIS_IN_TICKS",
              "TOT_SUBSET_ENROLL_BACKOFF_S", "TOT_REDUCED_TICK_INTERVAL_S"):
        monkeypatch.delenv(k, raising=False)
    s = get_settings()
    assert s.subset_gate_enabled is True
    assert s.living_bot_count == 10
    assert s.subset_recompute_interval_s == 60.0
    assert s.subset_hysteresis_out_ticks == 2
    assert s.subset_hysteresis_in_ticks == 1
    assert s.subset_enroll_backoff_s == 300.0
    assert s.reduced_tick_interval_s == 300.0


def test_living_bot_count_out_of_range_raises(monkeypatch):
    monkeypatch.setenv("TOT_LIVING_BOT_COUNT", "20")
    with pytest.raises(ValueError, match="must be 5-15"):
        get_settings()
    monkeypatch.setenv("TOT_LIVING_BOT_COUNT", "4")
    with pytest.raises(ValueError, match="must be 5-15"):
        get_settings()
