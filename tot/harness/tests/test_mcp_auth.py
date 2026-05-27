"""Unit tests for TokenStoreVerifier (MCP bearer adapter)."""

from __future__ import annotations

import pytest

from harness_daemon.auth import TokenStore
from harness_daemon.config import TokenRecord
from harness_daemon.mcp_auth import TokenStoreVerifier


def _store_with(token: str, identity: str, scope: list[str],
                bound_to_guid: int | None = None) -> TokenStore:
    return TokenStore([
        TokenRecord(token=token, identity=identity, scope=scope,
                    bound_to_guid=bound_to_guid),
    ])


@pytest.mark.asyncio
async def test_verify_known_token_returns_access_token() -> None:
    store = _store_with("good", "gm.tbrack", ["gm.*", "obs.*"])
    v = TokenStoreVerifier(store)
    tok = await v.verify_token("good")
    assert tok is not None
    assert tok.token == "good"
    assert tok.client_id == "gm.tbrack"
    assert set(tok.scopes) == {"gm.*", "obs.*"}


@pytest.mark.asyncio
async def test_verify_unknown_token_returns_none() -> None:
    store = _store_with("good", "gm.tbrack", ["gm.*"])
    v = TokenStoreVerifier(store)
    assert await v.verify_token("nope") is None


@pytest.mark.asyncio
async def test_verify_empty_token_returns_none() -> None:
    store = _store_with("good", "gm.tbrack", ["gm.*"])
    v = TokenStoreVerifier(store)
    assert await v.verify_token("") is None


@pytest.mark.asyncio
async def test_verify_preserves_bound_to_guid_via_resource_field() -> None:
    """bound_to_guid must travel with the AccessToken so dispatch_tool
    can enforce self-binding on the MCP side."""
    store = _store_with("good", "bot.X", ["bot.self.*"], bound_to_guid=42)
    v = TokenStoreVerifier(store)
    tok = await v.verify_token("good")
    assert tok is not None
    # We carry bound_to_guid in the resource field as `guid:42` — see
    # mcp_auth.TokenStoreVerifier for the encoding rationale.
    assert tok.resource == "guid:42"
