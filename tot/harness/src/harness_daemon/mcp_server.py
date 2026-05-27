"""FastMCP server factory.

Mounts the V1 tool registry onto a FastMCP instance with bearer-token
auth via `TokenStoreVerifier`. Each tool handler is a 3-line forward
into the shared `dispatch_tool` core in app.py — there is NO parallel
auth/audit/registry path.

Note on token plumbing (MCP SDK 1.27.1):
    `mcp.server.fastmcp.Context` does NOT expose `.access_token`
    directly. The access token is stored in a contextvar by the auth
    middleware and is retrieved via
    `mcp.server.auth.middleware.auth_context.get_access_token()`.
    That helper returns `AccessToken | None`. We re-resolve its `.token`
    back to a TokenRecord via the shared TokenStore so the dispatch
    core sees the same AuthResult shape as the HTTP surface.
"""

from __future__ import annotations

import time
import uuid
from typing import Any, Awaitable, Callable, Optional

from mcp.server.auth.middleware.auth_context import get_access_token
from mcp.server.auth.settings import AuthSettings
from mcp.server.fastmcp import Context, FastMCP
from mcp.server.transport_security import TransportSecuritySettings
from pydantic import AnyHttpUrl

from .ac_client import ACClient
from .audit import AuditEvent, AuditLogger
from .auth import AuthResult, TokenStore
from .config import TokenRecord
from .db_client import DBClient
from .mcp_auth import TokenStoreVerifier
from .registry import Registry
from .tool_schemas import TOOL_SCHEMAS

# Type of the shared dispatch core (see app.py: `dispatch_tool`).
DispatchFn = Callable[..., Awaitable[Any]]


def _auth_result_from_context(token_store: TokenStore) -> Optional[AuthResult]:
    """Re-resolve the current AccessToken's identity back to the TokenRecord.

    Uses `get_access_token()` from the MCP auth middleware contextvar,
    which is set per-request by the bearer auth pipeline that
    `TokenStoreVerifier` participates in.
    """
    access = get_access_token()
    if access is None:
        return None
    record: Optional[TokenRecord] = token_store.find(access.token)
    if record is None:
        return None
    return AuthResult(
        identity=record.identity,
        scope=list(record.scope),
        bound_to_guid=record.bound_to_guid,
        augmented=record.augmented,
    )


def build_mcp_server(
    *,
    token_store:    TokenStore,
    registry:       Registry,
    dispatch_fn:    DispatchFn,
    audit:          Optional[AuditLogger],
    ac_client:      Optional[ACClient]    = None,
    db_client:      Optional[DBClient]    = None,
    resource_url:   str                   = "http://192.168.1.3:8099/mcp",
    allowed_hosts:  Optional[list[str]]   = None,
) -> FastMCP:
    """Build a FastMCP server with every V1 tool registered.

    Parameters
    ----------
    token_store : the daemon's TokenStore (shared with HTTP surface)
    registry    : the V1 registry (shared with HTTP surface)
    dispatch_fn : `app.dispatch_tool` — injected to avoid a circular import
    audit       : the same AuditLogger the HTTP surface uses
    """
    verifier = TokenStoreVerifier(token_store)

    # DNS-rebind protection: an explicit allowlist lets us serve on
    # any LAN host (e.g. 192.168.1.3:8099) while still enforcing Host
    # header validation. Bearer auth is the primary gate; this is
    # defense-in-depth for the browser-attack vector that doesn't
    # apply to curl/Claude Code clients but is on by default.
    hosts = allowed_hosts if allowed_hosts is not None else ["127.0.0.1:*", "localhost:*"]
    origins = [f"http://{h}" for h in hosts] + [f"https://{h}" for h in hosts]
    transport_security = TransportSecuritySettings(
        enable_dns_rebinding_protection=True,
        allowed_hosts=hosts,
        allowed_origins=origins,
    )

    mcp = FastMCP(
        name="heimdal-harness",
        json_response=True,
        token_verifier=verifier,
        auth=AuthSettings(
            issuer_url=AnyHttpUrl(resource_url),
            resource_server_url=AnyHttpUrl(resource_url),
            required_scopes=[],
        ),
        transport_security=transport_security,
    )

    def _make_handler(tool_name: str, schema_cls):
        async def _handler(ctx, args) -> dict:
            request_id = f"mcp_{uuid.uuid4().hex[:12]}"
            t0 = time.perf_counter()
            auth = _auth_result_from_context(token_store)
            if auth is None:
                return {"ok": False, "error": "unauthorized"}

            args_dict = args.model_dump(exclude_none=True)
            outcome = await dispatch_fn(
                name=tool_name,
                args=args_dict,
                auth=auth,
                request_id=request_id,
                registry=registry,
                ac_client=ac_client,
                db_client=db_client,
            )
            latency_ms = int((time.perf_counter() - t0) * 1000)
            if audit is not None:
                audit.write(AuditEvent(
                    ts=time.time(), request_id=request_id, identity=auth.identity,
                    tool=tool_name, args_body=args_dict,
                    outcome=outcome.audit_outcome, status=outcome.status,
                    latency_ms=latency_ms, ac_latency_ms=outcome.ac_latency_ms,
                    error_detail=outcome.error_detail,
                    transport="mcp",
                ))
            return outcome.body

        # Set annotations explicitly so FastMCP's inspect.signature(eval_str=True)
        # can resolve them. `from __future__ import annotations` would
        # otherwise leave `schema_cls` as an unresolvable forward-ref string.
        _handler.__annotations__ = {
            "ctx":    Context,
            "args":   schema_cls,
            "return": dict,
        }
        _handler.__name__ = f"tool_{tool_name.replace('.', '_')}"
        return _handler

    for tool_name in registry.names():
        if tool_name not in TOOL_SCHEMAS:
            raise RuntimeError(
                f"tool {tool_name!r} in registry has no entry in TOOL_SCHEMAS — "
                f"add one to tool_schemas.py before shipping"
            )
        schema_cls, description = TOOL_SCHEMAS[tool_name]
        handler = _make_handler(tool_name, schema_cls)
        mcp.add_tool(
            handler,
            name=tool_name,
            description=description,
        )

    return mcp
