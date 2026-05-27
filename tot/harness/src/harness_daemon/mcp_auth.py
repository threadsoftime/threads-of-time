"""Adapt the daemon's TokenStore to MCP's TokenVerifier protocol.

MCP's auth model is OAuth-2.1-flavored: tokens carry `scopes` (a list
of strings) plus an opaque `client_id` (we use `identity`) and a
`resource` URI (we encode `bound_to_guid` here so the dispatch core can
still enforce self-binding on the MCP surface).

This adapter performs ONLY the existence check — every other gate
(scope match, self-binding, allowlist) lives in `dispatch_tool` and is
identical across HTTP and MCP.
"""

from __future__ import annotations

from typing import Optional

from mcp.server.auth.provider import AccessToken, TokenVerifier

from .auth import TokenStore


class TokenStoreVerifier(TokenVerifier):
    def __init__(self, store: TokenStore) -> None:
        self._store = store

    async def verify_token(self, token: str) -> Optional[AccessToken]:
        if not token:
            return None
        record = self._store.find(token)
        if record is None:
            return None
        resource = f"guid:{record.bound_to_guid}" if record.bound_to_guid is not None else None
        return AccessToken(
            token=token,
            client_id=record.identity,
            scopes=list(record.scope),
            expires_at=None,
            resource=resource,
        )
