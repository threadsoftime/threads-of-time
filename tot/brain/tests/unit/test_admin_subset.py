# SPDX-License-Identifier: GPL-2.0-or-later
"""Unit tests for the /admin/subset/* endpoints (Plan 3 T20, Spec §4.6)."""
from __future__ import annotations

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from unittest.mock import AsyncMock, MagicMock

from brain_sidecar.api import make_router
from brain_sidecar.models import PersonalityCard
from brain_sidecar.state import StateStore


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_personality() -> PersonalityCard:
    return PersonalityCard(
        name="Testbot",
        race="Human",
        **{"class": "Warrior"},
        backstory="test",
        talkativeness=0.5,
        courage=0.5,
        greed=0.5,
        attitude_to_master=0.5,
    )


def _make_stub_supervisor():
    class _Stub:
        def start(self, bot_guid: int) -> None:
            pass

        def list_active(self):
            return []

    return _Stub()


def _make_stub_gate(state_store, *, phase_b_enabled: bool = False):
    """Return a minimal SubsetGate-like object for API tests."""
    from brain_sidecar.subset_gate import SubsetGate, SubsetGateConfig

    async def _fetcher():
        from brain_sidecar.subset_gate import WorldSnapshot
        return WorldSnapshot(players=(), bots=())

    async def _enroll(g): pass
    async def _release(g): pass

    return SubsetGate(
        state_store=state_store,
        snapshot_fetcher=_fetcher,
        enroll_fn=_enroll,
        release_fn=_release,
        config=SubsetGateConfig(
            living_bot_count=10,
            recompute_interval_s=60.0,
            hysteresis_out_ticks=2,
            hysteresis_in_ticks=1,
            enroll_backoff_s=300.0,
            enabled=True,
            phase_b_enabled=phase_b_enabled,
        ),
    )


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture
def store(tmp_path):
    db_path = str(tmp_path / "state.sqlite")
    s = StateStore(db_path)
    s.migrate()
    yield s
    s.close()


@pytest.fixture
def test_client_with_state_store(store):
    """Yield (TestClient, StateStore) pair with a real StateStore + stub SubsetGate."""
    gate = _make_stub_gate(store)
    personality_cache = MagicMock()
    personality_cache.seed = AsyncMock(return_value=None)

    router = make_router(
        state_store=store,
        personality_cache=personality_cache,
        supervisor=_make_stub_supervisor(),
        brain_bearer="",  # auth disabled for tests
        subset_gate=gate,
    )
    app = FastAPI()
    app.include_router(router)
    client = TestClient(app, raise_server_exceptions=True)
    return client, store


# ---------------------------------------------------------------------------
# T20 tests
# ---------------------------------------------------------------------------

def test_pin_endpoint_persists_pin(test_client_with_state_store):
    client, state_store = test_client_with_state_store
    state_store.enroll(bot_guid=42, enrolled_at_ms=0,
                       personality_seed=_make_personality())
    r = client.post("/admin/subset/pin/42")
    assert r.status_code == 200
    body = r.json()
    assert body["ok"] is True
    assert body["bot_guid"] == 42
    assert body["pinned"] is True
    assert 42 in state_store.list_pinned()


def test_pin_endpoint_404_for_unknown_bot(test_client_with_state_store):
    client, state_store = test_client_with_state_store
    r = client.post("/admin/subset/pin/9999")
    assert r.status_code == 404


def test_unpin_endpoint_clears_pin(test_client_with_state_store):
    client, state_store = test_client_with_state_store
    state_store.enroll(bot_guid=42, enrolled_at_ms=0,
                       personality_seed=_make_personality())
    state_store.set_pin(42, True)
    r = client.post("/admin/subset/unpin/42")
    assert r.status_code == 200
    body = r.json()
    assert body["ok"] is True
    assert body["bot_guid"] == 42
    assert body["pinned"] is False
    assert state_store.list_pinned() == []


def test_unpin_endpoint_no_error_for_unknown_bot(test_client_with_state_store):
    """unpin is idempotent — no 404 for unknown bot_guid."""
    client, state_store = test_client_with_state_store
    r = client.post("/admin/subset/unpin/9999")
    assert r.status_code == 200
    assert r.json()["pinned"] is False


def test_snapshot_endpoint_returns_summary(test_client_with_state_store):
    client, state_store = test_client_with_state_store
    r = client.get("/admin/subset/snapshot")
    assert r.status_code == 200
    body = r.json()
    assert "currently_enrolled" in body
    assert "pinned" in body
    assert "config" in body


def test_snapshot_shows_enrolled_bots(test_client_with_state_store):
    client, state_store = test_client_with_state_store
    state_store.enroll(bot_guid=100, enrolled_at_ms=0,
                       personality_seed=_make_personality())
    r = client.get("/admin/subset/snapshot")
    assert r.status_code == 200
    body = r.json()
    guids = [row["bot_guid"] for row in body["currently_enrolled"]]
    assert 100 in guids


def test_snapshot_shows_pinned_bots(test_client_with_state_store):
    client, state_store = test_client_with_state_store
    state_store.enroll(bot_guid=200, enrolled_at_ms=0,
                       personality_seed=_make_personality())
    state_store.set_pin(200, True)
    r = client.get("/admin/subset/snapshot")
    assert r.status_code == 200
    assert 200 in r.json()["pinned"]


def test_snapshot_config_reflects_gate_config(test_client_with_state_store):
    client, state_store = test_client_with_state_store
    r = client.get("/admin/subset/snapshot")
    body = r.json()
    assert body["config"]["living_bot_count"] == 10
    assert body["config"]["recompute_interval_s"] == 60.0
    assert body["config"]["phase_b_enabled"] is False


@pytest.mark.asyncio
async def test_recompute_endpoint_returns_decision(store):
    """POST /admin/subset/recompute invokes _recompute_and_apply and returns structured result."""
    gate = _make_stub_gate(store)
    personality_cache = MagicMock()
    personality_cache.seed = AsyncMock(return_value=None)

    router = make_router(
        state_store=store,
        personality_cache=personality_cache,
        supervisor=_make_stub_supervisor(),
        brain_bearer="",
        subset_gate=gate,
    )
    from fastapi import FastAPI
    from httpx import AsyncClient, ASGITransport

    app = FastAPI()
    app.include_router(router)

    async with AsyncClient(transport=ASGITransport(app=app), base_url="http://test") as client:
        r = await client.post("/admin/subset/recompute")

    assert r.status_code == 200
    body = r.json()
    assert "target" in body
    assert "to_enroll" in body
    assert "to_release" in body
    assert "to_full" in body
    assert "to_reduced" in body


def test_recompute_503_when_no_gate(store):
    """POST /admin/subset/recompute returns 503 when subset_gate is None."""
    personality_cache = MagicMock()
    router = make_router(
        state_store=store,
        personality_cache=personality_cache,
        supervisor=_make_stub_supervisor(),
        brain_bearer="",
        subset_gate=None,
    )
    app = FastAPI()
    app.include_router(router)
    client = TestClient(app, raise_server_exceptions=False)
    r = client.post("/admin/subset/recompute")
    assert r.status_code == 503


def test_snapshot_no_gate_returns_empty_config(store):
    """GET /admin/subset/snapshot works even when subset_gate=None."""
    personality_cache = MagicMock()
    router = make_router(
        state_store=store,
        personality_cache=personality_cache,
        supervisor=_make_stub_supervisor(),
        brain_bearer="",
        subset_gate=None,
    )
    app = FastAPI()
    app.include_router(router)
    client = TestClient(app)
    r = client.get("/admin/subset/snapshot")
    assert r.status_code == 200
    body = r.json()
    assert body["config"] == {}
