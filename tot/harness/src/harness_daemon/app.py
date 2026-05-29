"""FastAPI application wiring auth, registry, AC client, audit."""

from __future__ import annotations

import os
import time
import uuid
from dataclasses import dataclass
from typing import Any, Optional

from fastapi import FastAPI, Request, Response
from fastapi.responses import JSONResponse

from .ac_client import ACClient, ACClientError
from .audit import AuditEvent, AuditLogger
from .db_client import DBClient, UnknownTemplate, BadParams
from .auth import (
    AuthError,
    AuthResult,
    TokenStore,
    authenticate_bearer,
    is_self_scope,
    scope_allows,
    _pattern_matches,
)
from .config import DaemonConfig
from .registry import (
    Registry,
    SelfBindingViolation,
    ToolEntry,
    ToolNotFound,
    build_v1_registry,
)


def _json_error(status: int, code: str, detail: str = "", **extra: Any) -> JSONResponse:
    body = {"ok": False, "error": code, "detail": detail, **extra}
    return JSONResponse(status_code=status, content=body)


@dataclass
class DispatchOutcome:
    """Result of dispatch_tool: status + body + audit metadata.

    Both surfaces (HTTP, MCP) translate this into their transport's
    native shape. The body shape MUST match what the V1.x HTTP
    endpoints emit today (callers depend on it).
    """
    status:         int
    body:           dict
    audit_outcome:  str
    error_detail:   str = ""
    ac_latency_ms:  int | None = None


def _find_matching_pattern(scope: list[str], tool: str) -> Optional[str]:
    """Return the first scope pattern that matches `tool`, else None."""
    for pattern in scope:
        if _pattern_matches(pattern, tool):
            return pattern
    return None


async def dispatch_tool(
    *,
    name:        str,
    args:        dict,
    auth:        AuthResult,
    request_id:  str,
    registry:    Registry,
    ac_client:   ACClient,
    db_client:   Optional[DBClient],
) -> DispatchOutcome:
    """Shared dispatch core for HTTP and MCP surfaces.

    Caller MUST have already authenticated the bearer and parsed `args`
    as JSON. This function performs:
      1. scope-pattern match
      2. registry lookup
      3. self-binding (when matched pattern is `<ns>.self.*`)
      4. `gm.run_console` allowlist
      5. daemon-direct (`obs.query_db`) OR AC forward
    It does NOT write to the audit log — the caller does, because the
    caller has the latency, transport, and identity context.

    Body shape on success:
        {"ok": True, "result": {...}}
    Body shape on error:
        {"ok": False, "error": "<code>", "detail": "...", **extra}
    """
    # --- Scope match ---
    matched_pattern = _find_matching_pattern(auth.scope, name)
    if matched_pattern is None:
        return DispatchOutcome(
            status=403,
            body={"ok": False, "error": "scope_denied", "detail": "",
                  "needed": name, "scope": auth.scope},
            audit_outcome="scope_denied",
        )

    # --- Tool lookup ---
    try:
        entry: ToolEntry = registry.find(name)
    except ToolNotFound:
        return DispatchOutcome(
            status=404,
            body={"ok": False, "error": "unknown_tool", "detail": ""},
            audit_outcome="unknown_tool",
        )

    # --- Self-binding ---
    if is_self_scope(matched_pattern):
        if auth.bound_to_guid is None:
            return DispatchOutcome(
                status=403,
                body={"ok": False, "error": "not_bound",
                      "detail": "self-scope token has no bound_to_guid"},
                audit_outcome="not_bound",
                error_detail="self-scope token has no bound_to_guid",
            )
        try:
            entry.check_self_binding(args, auth.bound_to_guid)
        except SelfBindingViolation as e:
            return DispatchOutcome(
                status=403,
                body={"ok": False, "error": "not_bound", "detail": str(e)},
                audit_outcome="not_bound",
                error_detail=str(e),
            )

    # --- Allowlist for gm.run_console ---
    if name == "gm.run_console":
        cmd = args.get("command", "")
        allowed_prefixes = (".lookup", ".gobject", ".npc", ".bracketsets")
        if not any(cmd.startswith(p) for p in allowed_prefixes):
            detail = f"command not allowlisted: {cmd[:40]}"
            return DispatchOutcome(
                status=403,
                body={"ok": False, "error": "scope_denied",
                      "detail": f"run_console only accepts: {allowed_prefixes}"},
                audit_outcome="scope_denied",
                error_detail=detail,
            )

    # --- Daemon-direct path ---
    if not entry.forwards_to_ac:
        if name != "obs.query_db":
            return DispatchOutcome(
                status=501,
                body={"ok": False, "error": "not_implemented",
                      "detail": f"daemon-direct tool '{name}' not yet wired"},
                audit_outcome="not_implemented",
            )
        if db_client is None:
            return DispatchOutcome(
                status=503,
                body={"ok": False, "error": "unavailable",
                      "detail": "db_client not configured"},
                audit_outcome="db_not_configured",
                error_detail="db_client not configured",
            )
        try:
            tpl = args.get("template_name")
            tpl_params = args.get("params", {})
            rows = await db_client.query(tpl, tpl_params)
        except UnknownTemplate as e:
            return DispatchOutcome(
                status=400,
                body={"ok": False, "error": "bad_request",
                      "detail": f"unknown template: {e!s}"},
                audit_outcome="bad_request",
                error_detail=f"unknown template: {e!s}",
            )
        except BadParams as e:
            return DispatchOutcome(
                status=400,
                body={"ok": False, "error": "bad_request", "detail": str(e)},
                audit_outcome="bad_request",
                error_detail=str(e),
            )
        return DispatchOutcome(
            status=200,
            body={"ok": True, "result": {"rows": rows, "row_count": len(rows)}},
            audit_outcome="ok",
        )

    # --- Forward to AC ---
    try:
        ac_resp = await ac_client.dispatch(
            tool=name, args=args,
            request_id=request_id, identity=auth.identity,
        )
    except ACClientError as e:
        return DispatchOutcome(
            status=503,
            body={"ok": False, "error": "unavailable", "detail": str(e)},
            audit_outcome="ac_unreachable",
            error_detail=str(e),
        )

    outcome = "ok" if ac_resp.status == 200 else "ac_error"
    return DispatchOutcome(
        status=ac_resp.status,
        body=ac_resp.body,
        audit_outcome=outcome,
        ac_latency_ms=ac_resp.ac_latency_ms,
        error_detail=ac_resp.body.get("detail", "") if ac_resp.status != 200 else "",
    )


def build_app(cfg: DaemonConfig,
              ac_client: ACClient,
              db_client: Optional[DBClient] = None) -> FastAPI:
    """Construct a FastAPI app for the given config + (possibly mocked) AC client."""
    import contextlib
    from .mcp_server import build_mcp_server

    token_store = TokenStore(cfg.tokens)
    registry = build_v1_registry()
    audit = AuditLogger(cfg.audit_path)

    # MCP host allowlist: client Host headers must match one of these
    # entries (with FastMCP's `*` wildcard for ports). The daemon may be
    # reached on an external host/IP that differs from its bind address
    # (`cfg.listen_address` is e.g. `0.0.0.0:8099` but clients may connect
    # to `realm.example.com:8099`); operators add such hostnames via the
    # HARNESS_EXTRA_ALLOWED_HOSTS env var (comma-separated, `*` = any port).
    mcp_allowed_hosts = [
        "127.0.0.1:*",
        "localhost:*",
        cfg.listen_address,
    ]
    _extra_hosts = os.environ.get("HARNESS_EXTRA_ALLOWED_HOSTS", "")
    mcp_allowed_hosts += [h.strip() for h in _extra_hosts.split(",") if h.strip()]
    mcp_server = build_mcp_server(
        token_store=token_store,
        registry=registry,
        dispatch_fn=dispatch_tool,
        audit=audit,
        ac_client=ac_client,
        db_client=db_client,
        resource_url=f"http://{cfg.listen_address}/mcp",
        allowed_hosts=mcp_allowed_hosts,
    )

    # FastMCP's `session_manager` accessor raises RuntimeError if invoked
    # before `streamable_http_app()` has been called. Call it once now so
    # the lifespan body below can safely reference `session_manager`.
    _streamable_app = mcp_server.streamable_http_app()

    @contextlib.asynccontextmanager
    async def lifespan(_app: FastAPI):
        # FastMCP's streamable-HTTP transport requires its session
        # manager to be running for the duration of the app.
        async with mcp_server.session_manager.run():
            yield

    app = FastAPI(title="harness-daemon", version="0.2.0", lifespan=lifespan)
    app.mount("/mcp", _streamable_app)

    async def _handle_tool_call(
        request: Request,
        name: str,
        endpoint: str,  # "tools" or "observations" — for audit
    ) -> Response:
        request_id = request.headers.get("X-Request-Id") or f"req_{uuid.uuid4().hex[:12]}"
        t0 = time.perf_counter()

        # --- Auth ---
        try:
            auth: AuthResult = authenticate_bearer(
                token_store, request.headers.get("Authorization")
            )
        except AuthError as e:
            audit.write(AuditEvent(
                ts=time.time(), request_id=request_id, identity="unknown",
                tool=name, args_body={}, outcome="unauthorized",
                status=401, latency_ms=int((time.perf_counter() - t0) * 1000),
                error_detail=e.detail,
            ))
            return _json_error(401, "unauthorized")

        # --- Body parse ---
        try:
            args = await request.json() if await request.body() else {}
        except ValueError as e:
            audit.write(AuditEvent(
                ts=time.time(), request_id=request_id, identity=auth.identity,
                tool=name, args_body={}, outcome="bad_request", status=400,
                latency_ms=int((time.perf_counter() - t0) * 1000),
                error_detail=f"json parse: {e!s}",
            ))
            return _json_error(400, "bad_request", detail=str(e))

        # --- Shared dispatch ---
        result = await dispatch_tool(
            name=name, args=args, auth=auth, request_id=request_id,
            registry=registry, ac_client=ac_client, db_client=db_client,
        )

        latency_ms = int((time.perf_counter() - t0) * 1000)
        audit.write(AuditEvent(
            ts=time.time(), request_id=request_id, identity=auth.identity,
            tool=name, args_body=args, outcome=result.audit_outcome,
            status=result.status, latency_ms=latency_ms,
            ac_latency_ms=result.ac_latency_ms,
            error_detail=result.error_detail,
        ))

        # Successful AC dispatch OR AC returned a non-200 — both surface
        # the meta fields per V1 wire contract. Other errors (auth,
        # scope, registry, not_bound, ac_unreachable, db config) emit
        # the raw error envelope from dispatch_tool.
        if result.audit_outcome in ("ok", "ac_error"):
            return JSONResponse(
                status_code=result.status,
                content={
                    **result.body,
                    "request_id":    request_id,
                    "identity":      auth.identity,
                    "latency_ms":    latency_ms,
                    "ac_latency_ms": result.ac_latency_ms,
                },
            )

        # Error path: emit the body as-is, with optional Retry-After.
        headers = {"Retry-After": "1"} if result.audit_outcome == "ac_unreachable" else {}
        return JSONResponse(status_code=result.status, content=result.body, headers=headers)

    @app.post("/v1/tools/{name}")
    async def tools_endpoint(name: str, request: Request) -> Response:
        return await _handle_tool_call(request, name, "tools")

    @app.post("/v1/observations/{name}")
    async def observations_endpoint(name: str, request: Request) -> Response:
        return await _handle_tool_call(request, name, "observations")

    @app.get("/v1/health")
    async def health() -> Response:
        ac_ok = await ac_client.health()
        return JSONResponse(content={"ok": True, "ac_bridge_reachable": ac_ok})

    @app.get("/v1/audit")
    async def audit_replay(since: float = 0.0) -> Response:
        # V1: file streaming with simple `since` filter. V2 upgrades to SSE.
        from pathlib import Path
        import json as _json
        p = Path(cfg.audit_path)
        if not p.exists():
            return JSONResponse(content={"events": []})
        out: list[dict] = []
        with p.open() as fh:
            for line in fh:
                try:
                    rec = _json.loads(line)
                except ValueError:
                    continue
                if rec.get("ts", 0) >= since:
                    out.append(rec)
        return JSONResponse(content={"events": out})

    return app
