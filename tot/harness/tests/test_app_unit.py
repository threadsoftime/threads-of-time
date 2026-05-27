"""Unit tests for the FastAPI app with the AC client mocked."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any
from unittest.mock import AsyncMock

import pytest
from fastapi.testclient import TestClient

from harness_daemon.app import build_app
from harness_daemon.config import DaemonConfig, TokenRecord


@dataclass
class FakeACResponse:
    status: int
    body: dict
    ac_latency_ms: int = 5


def _make_config() -> DaemonConfig:
    return DaemonConfig(
        max_augmented_bots=1,
        ac_bridge_url="http://127.0.0.1:8091",
        audit_path="/tmp/harness-daemon-test-audit.jsonl",
        listen_address="0.0.0.0:8090",
        tokens=[
            TokenRecord(token="ok-tok", identity="ok.user",
                        scope=["gm.*", "obs.*"]),
            TokenRecord(token="bot-tok", identity="bot.X",
                        scope=["bot.self.*"], bound_to_guid=42, augmented=True),
        ],
    )


@pytest.fixture
def app_and_mock(tmp_path):
    cfg = _make_config()
    cfg = DaemonConfig(
        max_augmented_bots=cfg.max_augmented_bots,
        ac_bridge_url=cfg.ac_bridge_url,
        audit_path=str(tmp_path / "audit.jsonl"),
        listen_address=cfg.listen_address,
        tokens=cfg.tokens,
    )
    mock_ac = AsyncMock()
    mock_ac.dispatch = AsyncMock()
    mock_ac.health = AsyncMock(return_value=True)
    mock_ac.close = AsyncMock()

    app = build_app(cfg, ac_client=mock_ac)
    return app, mock_ac


def test_health_returns_200(app_and_mock) -> None:
    app, _ = app_and_mock
    with TestClient(app) as client:
        r = client.get("/v1/health")
        assert r.status_code == 200
        assert r.json()["ok"] is True


def test_dispatch_unauthorized_no_bearer(app_and_mock) -> None:
    app, _ = app_and_mock
    with TestClient(app) as client:
        r = client.post("/v1/tools/obs.ping", json={})
        assert r.status_code == 401
        assert r.json()["error"] == "unauthorized"


def test_dispatch_unauthorized_bad_token(app_and_mock) -> None:
    app, _ = app_and_mock
    with TestClient(app) as client:
        r = client.post("/v1/tools/obs.ping", json={},
                        headers={"Authorization": "Bearer nope"})
        assert r.status_code == 401


def test_dispatch_scope_denied(app_and_mock) -> None:
    app, _ = app_and_mock
    with TestClient(app) as client:
        # bot-tok has bot.self.* only; gm.additem requires gm.*
        r = client.post(
            "/v1/tools/gm.additem",
            json={"target_guid": 42, "item_entry": 90000, "count": 1},
            headers={"Authorization": "Bearer bot-tok"},
        )
        assert r.status_code == 403
        assert r.json()["error"] == "scope_denied"


def test_dispatch_self_binding_mismatch(app_and_mock) -> None:
    app, _ = app_and_mock
    with TestClient(app) as client:
        # bot-tok bound_to_guid=42 calling bot.set_goal with bot_guid=999
        r = client.post(
            "/v1/tools/bot.set_goal",
            json={"bot_guid": 999, "goal": {}},
            headers={"Authorization": "Bearer bot-tok"},
        )
        assert r.status_code == 403
        assert r.json()["error"] == "not_bound"


def test_dispatch_unknown_tool_to_authorized_caller(app_and_mock) -> None:
    app, _ = app_and_mock
    with TestClient(app) as client:
        r = client.post(
            "/v1/tools/gm.does_not_exist",
            json={},
            headers={"Authorization": "Bearer ok-tok"},
        )
        # Caller has gm.* so they see 404, not 403.
        assert r.status_code == 404
        assert r.json()["error"] == "unknown_tool"


def test_dispatch_unknown_tool_to_unscoped_caller(app_and_mock) -> None:
    app, _ = app_and_mock
    with TestClient(app) as client:
        r = client.post(
            "/v1/tools/gm.does_not_exist",
            json={},
            headers={"Authorization": "Bearer bot-tok"},  # no gm.* scope
        )
        # No matching scope → 403 (don't enumerate tools).
        assert r.status_code == 403


def test_dispatch_happy_path_forwards_to_ac(app_and_mock) -> None:
    app, mock_ac = app_and_mock
    mock_ac.dispatch.return_value = FakeACResponse(
        status=200,
        body={"ok": True, "result": {"pong": True, "ts_ms": 1779000000123},
              "tick_wait_ms": 30, "executor_ms": 1},
    )
    with TestClient(app) as client:
        r = client.post(
            "/v1/tools/obs.ping",
            json={},
            headers={"Authorization": "Bearer ok-tok"},
        )
        assert r.status_code == 200
        body = r.json()
        assert body["ok"] is True
        assert body["result"]["pong"] is True
        assert body["identity"] == "ok.user"
        assert "request_id" in body

        mock_ac.dispatch.assert_awaited_once()
        args, kwargs = mock_ac.dispatch.call_args
        assert kwargs["tool"] == "obs.ping"
        assert kwargs["identity"] == "ok.user"


def test_ac_error_response_includes_meta_fields(app_and_mock) -> None:
    """Wire contract: AC-returned non-200 bodies keep request_id,
    identity, latency_ms, ac_latency_ms (V1 contract; regression
    guard for the T3 dispatch_tool refactor)."""
    app, mock_ac = app_and_mock
    mock_ac.dispatch.return_value = FakeACResponse(
        status=422,
        body={"ok": False, "error": "bridge_validation", "detail": "bad arg"},
        ac_latency_ms=3,
    )
    with TestClient(app) as client:
        r = client.post(
            "/v1/tools/obs.ping",
            json={},
            headers={"Authorization": "Bearer ok-tok"},
        )
        assert r.status_code == 422
        body = r.json()
        assert body["error"] == "bridge_validation"
        assert "request_id" in body
        assert body["identity"] == "ok.user"
        assert "latency_ms" in body
        assert body["ac_latency_ms"] == 3
