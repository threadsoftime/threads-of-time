"""Tests for bearer-token auth + scope-glob matching + self-binding."""

from __future__ import annotations

import pytest

from harness_daemon.auth import (
    AuthError,
    AuthResult,
    TokenStore,
    authenticate_bearer,
    scope_allows,
    is_self_scope,
)
from harness_daemon.config import TokenRecord


@pytest.fixture
def store() -> TokenStore:
    return TokenStore([
        TokenRecord(token="gm-tok", identity="gm.tbrack",
                    scope=["gm.*", "obs.*"]),
        TokenRecord(token="bot-tok", identity="bot.Krak",
                    scope=["bot.self.*", "obs.self.*"],
                    bound_to_guid=12345, augmented=True),
    ])


def test_bearer_happy_path(store: TokenStore) -> None:
    r = authenticate_bearer(store, "Bearer gm-tok")
    assert r.identity == "gm.tbrack"
    assert r.scope == ["gm.*", "obs.*"]


def test_bearer_unknown_token(store: TokenStore) -> None:
    with pytest.raises(AuthError, match="unauthorized"):
        authenticate_bearer(store, "Bearer nope")


def test_bearer_missing_header(store: TokenStore) -> None:
    with pytest.raises(AuthError, match="unauthorized"):
        authenticate_bearer(store, None)


def test_bearer_malformed(store: TokenStore) -> None:
    with pytest.raises(AuthError, match="unauthorized"):
        authenticate_bearer(store, "gm-tok")  # missing "Bearer "
    with pytest.raises(AuthError, match="unauthorized"):
        authenticate_bearer(store, "Bearer   ")  # empty token


def test_scope_allows_exact_match() -> None:
    assert scope_allows(["gm.additem"], "gm.additem") is True


def test_scope_allows_single_segment_glob() -> None:
    assert scope_allows(["gm.*"], "gm.additem") is True
    assert scope_allows(["gm.*"], "gm.teleport") is True


def test_scope_glob_does_not_cross_dots() -> None:
    # gm.* matches one segment only; gm.sub.foo is NOT matched.
    assert scope_allows(["gm.*"], "gm.sub.foo") is False


def test_scope_denies_other_namespace() -> None:
    assert scope_allows(["gm.*"], "bot.set_goal") is False


def test_scope_self_glob_matches_namespace() -> None:
    # `bot.self.*` matches `bot.<anything>` at the pattern level;
    # the binding check is a separate step (is_self_scope).
    assert scope_allows(["bot.self.*"], "bot.set_goal") is True


def test_is_self_scope_detection() -> None:
    assert is_self_scope("bot.self.*") is True
    assert is_self_scope("obs.self.*") is True
    assert is_self_scope("gm.*") is False
    assert is_self_scope("bot.*") is False
