#!/usr/bin/env python3
"""Phase 12 parity gate: empirically prove harness-rs is a byte/shape drop-in
for the Python daemon on BOTH the REST and MCP surfaces.

Python daemon is the ORACLE.  Where they differ, the Rust side is wrong.

Usage:
  /path/to/.venv/bin/python run_parity.py [--rust-binary /path/to/harness-rs]

Exit codes:
  0  all cases PASS
  1  one or more cases FAIL or a daemon failed to start
"""
from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Optional

import httpx

# MCP client — from the Python harness venv that supplies mcp 1.27.1
try:
    from mcp import ClientSession
    from mcp.client.streamable_http import streamablehttp_client
    MCP_AVAILABLE = True
except ImportError:
    MCP_AVAILABLE = False
    print("WARNING: mcp package not available — MCP battery will be SKIPPED", file=sys.stderr)

# ── locate things ─────────────────────────────────────────────────────────────

SCRIPT_DIR  = Path(__file__).parent
PYTHON_DAEMON_BIN = Path(
    "/Users/tbrack/Documents/Projects/threads-of-time/tot/harness/.venv/bin/harness-daemon"
)
RUST_BINARY_DEFAULT = (
    SCRIPT_DIR.parent.parent / "target" / "debug" / "harness-rs"
)

# ── helpers ───────────────────────────────────────────────────────────────────

def free_port() -> int:
    """Return an unused TCP port on 127.0.0.1."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def sha256_args(body: Any) -> str:
    canonical = json.dumps(body, sort_keys=True, separators=(",", ":"), default=str)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def wait_for_healthy(base_url: str, timeout: float = 15.0, label: str = "") -> None:
    """Poll GET /v1/health until 200, or raise RuntimeError on timeout."""
    deadline = time.monotonic() + timeout
    last_err: str = ""
    while time.monotonic() < deadline:
        try:
            r = httpx.get(f"{base_url}/v1/health", timeout=2.0)
            if r.status_code == 200:
                return
            last_err = f"status={r.status_code}"
        except (httpx.ConnectError, httpx.ReadError, httpx.RemoteProtocolError) as e:
            last_err = str(e)
        time.sleep(0.25)
    raise RuntimeError(
        f"[{label}] daemon did not become healthy at {base_url} within {timeout}s: {last_err}"
    )


def read_last_audit_line(audit_path: str) -> Optional[dict]:
    """Read the last non-empty line from the JSONL audit file."""
    p = Path(audit_path)
    if not p.exists():
        return None
    lines = [l.strip() for l in p.read_text().splitlines() if l.strip()]
    if not lines:
        return None
    try:
        return json.loads(lines[-1])
    except json.JSONDecodeError:
        return None


def normalize_body(body: dict) -> dict:
    """Remove volatile fields (request_id value, latency values) for comparison.

    Keeps the KEYS but replaces volatile values with type-sentinels.
    """
    out = {}
    for k, v in body.items():
        if k == "request_id":
            out[k] = "__PRESENT_STRING__"
        elif k == "latency_ms":
            out[k] = "__PRESENT_NUMBER__"
        elif k == "ac_latency_ms":
            # Only normalize if both are numbers (value differs by timing).
            # If one is null and other is non-null, that's a real difference.
            out[k] = "__PRESENT_NUMBER__" if v is not None else None
        else:
            out[k] = v
    return out


def bodies_match(py_body: dict, rs_body: dict, label: str = "") -> tuple[bool, str]:
    """Compare two normalized bodies.  Returns (ok, diff_msg)."""
    pn = normalize_body(py_body)
    rn = normalize_body(rs_body)
    if pn == rn:
        return True, ""
    all_keys = set(pn) | set(rn)
    diffs = []
    for k in sorted(all_keys):
        pv = pn.get(k, "<MISSING>")
        rv = rn.get(k, "<MISSING>")
        if pv != rv:
            diffs.append(f"  {k}: py={pv!r}  rs={rv!r}")
    if not diffs:
        return True, ""
    return False, f"Body mismatch for {label!r}:\n" + "\n".join(diffs)


def audits_match(py_audit: Optional[dict], rs_audit: Optional[dict], label: str = "") -> tuple[bool, str]:
    """Compare audit lines on the fields that must be equal."""
    if py_audit is None and rs_audit is None:
        return True, ""
    if py_audit is None:
        return False, f"Audit mismatch for {label!r}: python=None, rust has entry"
    if rs_audit is None:
        return False, f"Audit mismatch for {label!r}: rust=None, python has entry"

    # Fields that MUST be identical
    fields_to_compare = [
        "args_sha256", "outcome", "status", "identity", "tool", "transport",
    ]
    diffs = []
    for f in fields_to_compare:
        pv = py_audit.get(f, "<MISSING>")
        rv = rs_audit.get(f, "<MISSING>")
        if pv != rv:
            diffs.append(f"  {f}: py={pv!r}  rs={rv!r}")

    # ac_latency_ms comparison:
    # Python uses `int = 0` as default (always present; 0 for non-AC calls).
    # Rust uses `Option<u64>` with `None` serialized as `null` for non-AC calls.
    # For AC-forwarding calls: both should be present and be integers (including 0
    # for sub-millisecond responses from a local mock).
    # Accepted deviation: py=0, rs=null (or py=integer, rs=integer with different values).
    # What we assert: if the OUTCOME is "ok" or "ac_error" (AC was called),
    # BOTH must have a non-null numeric value for ac_latency_ms.
    outcome = py_audit.get("outcome", "")
    py_aclat = py_audit.get("ac_latency_ms")
    rs_aclat = rs_audit.get("ac_latency_ms")

    if outcome in ("ok", "ac_error"):
        # AC was called — both must have numeric ac_latency_ms
        py_ok = isinstance(py_aclat, (int, float))
        rs_ok = isinstance(rs_aclat, (int, float))
        if not py_ok or not rs_ok:
            diffs.append(
                f"  ac_latency_ms: for outcome={outcome!r}, expected numbers; "
                f"py={py_aclat!r} rs={rs_aclat!r}"
            )
    # For non-AC outcomes: py=0 vs rs=null is ACCEPTED (documented deviation)

    if diffs:
        return False, f"Audit mismatch for {label!r}:\n" + "\n".join(diffs)
    return True, ""


# ── PASS/FAIL tracking ────────────────────────────────────────────────────────

class Results:
    def __init__(self) -> None:
        self.cases: list[tuple[str, bool, str]] = []

    def record(self, label: str, ok: bool, msg: str = "") -> None:
        self.cases.append((label, ok, msg))
        status = "PASS" if ok else "FAIL"
        print(f"  [{status}] {label}", end="")
        if not ok and msg:
            indented = msg.replace("\n", "\n         ")
            print(f"\n         {indented}", end="")
        print()

    def summary(self) -> bool:
        passed = sum(1 for _, ok, _ in self.cases if ok)
        failed = sum(1 for _, ok, _ in self.cases if not ok)
        print(f"\n{'='*60}")
        print(f"PARITY GATE: {passed} PASS, {failed} FAIL")
        print("="*60)
        if failed:
            print("\nFailed cases:")
            for label, ok, msg in self.cases:
                if not ok:
                    print(f"  - {label}")
        return failed == 0


# ── fixture + daemon lifecycle ────────────────────────────────────────────────

def write_fixture(listen_address: str, ac_bridge_url: str, audit_path: str, out_path: str) -> None:
    yaml = f"""# Auto-generated parity fixture
max_augmented_bots: 10
ac_bridge_url: "{ac_bridge_url}"
audit_path: "{audit_path}"
listen_address: "{listen_address}"

tokens:
  - token: "parity-all"
    identity: "parity.user"
    scope:
      - "gm.*"
      - "bot.*"
      - "obs.*"
      - "event.*"
      - "lfg.*"
      - "memory.*"
    note: "parity gate: full-scope token"

  - token: "parity-self"
    identity: "bot.self"
    scope:
      - "bot.self.*"
      - "obs.self.*"
    bound_to_guid: 777
    augmented: true
    note: "parity gate: self-bound token"

  - token: "parity-obs"
    identity: "parity.obs"
    scope:
      - "obs.*"
    note: "parity gate: obs-only token"
"""
    Path(out_path).write_text(yaml)


def start_python_daemon(fixture_path: str) -> subprocess.Popen:
    return subprocess.Popen(
        [str(PYTHON_DAEMON_BIN), "serve", "--config", fixture_path],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def start_rust_daemon(binary: Path, fixture_path: str) -> subprocess.Popen:
    return subprocess.Popen(
        [str(binary), "serve", "--config", fixture_path],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def kill_proc(proc: Optional[subprocess.Popen], label: str) -> None:
    if proc is None:
        return
    try:
        proc.terminate()
        proc.wait(timeout=5)
    except Exception:
        try:
            proc.kill()
        except Exception:
            pass
    try:
        _, stderr = proc.communicate(timeout=2)
        if stderr:
            tail = stderr.decode(errors="replace")[-500:]
            if tail.strip():
                print(f"  [{label} stderr tail]: {tail}", file=sys.stderr)
    except Exception:
        pass


# ── REST battery ──────────────────────────────────────────────────────────────

def hpost(url: str, json_body: Optional[Any] = None, data: Optional[bytes] = None,
          headers: Optional[dict] = None, timeout: float = 10.0) -> httpx.Response:
    with httpx.Client(timeout=timeout) as client:
        h = headers or {}
        if data is not None:
            return client.post(url, content=data, headers=h)
        return client.post(url, json=json_body, headers=h)


def hget(url: str, headers: Optional[dict] = None, timeout: float = 10.0) -> httpx.Response:
    with httpx.Client(timeout=timeout) as client:
        return client.get(url, headers=headers or {})


def run_rest_battery(
    results: Results,
    py_url: str,
    rs_url: str,
    py_audit: str,
    rs_audit: str,
    mock_ac,  # MockAC instance
) -> Any:
    auth_all  = {"Authorization": "Bearer parity-all",  "Content-Type": "application/json"}
    auth_self = {"Authorization": "Bearer parity-self", "Content-Type": "application/json"}
    auth_obs  = {"Authorization": "Bearer parity-obs",  "Content-Type": "application/json"}
    no_auth   = {"Content-Type": "application/json"}

    print("\n--- REST battery ---")

    # ── (a) ok-forward obs.ping ───────────────────────────────────────────────

    py_r = hpost(f"{py_url}/v1/tools/obs.ping", json_body={}, headers=auth_all)
    rs_r = hpost(f"{rs_url}/v1/tools/obs.ping", json_body={}, headers=auth_all)

    ok = (py_r.status_code == rs_r.status_code == 200)
    body_ok, body_msg = bodies_match(py_r.json(), rs_r.json(), "obs.ping ok")
    results.record("REST (a) obs.ping → 200 + body match", ok and body_ok, body_msg)

    py_al = read_last_audit_line(py_audit)
    rs_al = read_last_audit_line(rs_audit)
    audit_ok, audit_msg = audits_match(py_al, rs_al, "obs.ping ok audit")
    results.record("REST (a) obs.ping → audit match", audit_ok, audit_msg)

    # ── (b) scope_denied: parity-obs calling gm.teleport ─────────────────────

    py_r = hpost(f"{py_url}/v1/tools/gm.teleport",
                 json_body={"target_guid": 1, "map": 0, "x": 0.0, "y": 0.0, "z": 0.0},
                 headers=auth_obs)
    rs_r = hpost(f"{rs_url}/v1/tools/gm.teleport",
                 json_body={"target_guid": 1, "map": 0, "x": 0.0, "y": 0.0, "z": 0.0},
                 headers=auth_obs)
    ok = (py_r.status_code == rs_r.status_code == 403)
    body_ok, body_msg = bodies_match(py_r.json(), rs_r.json(), "scope_denied")
    results.record("REST (b) scope_denied → 403 + body match", ok and body_ok, body_msg)

    py_al = read_last_audit_line(py_audit)
    rs_al = read_last_audit_line(rs_audit)
    audit_ok, audit_msg = audits_match(py_al, rs_al, "scope_denied audit")
    results.record("REST (b) scope_denied → audit match", audit_ok, audit_msg)

    # ── (c) unknown_tool ──────────────────────────────────────────────────────
    # Use obs.does_not_exist — scope matches obs.* but tool is not in the registry.
    # nope.tool would fail scope check first (no nope.* scope), returning 403.

    py_r = hpost(f"{py_url}/v1/tools/obs.does_not_exist", json_body={}, headers=auth_all)
    rs_r = hpost(f"{rs_url}/v1/tools/obs.does_not_exist", json_body={}, headers=auth_all)
    ok = (py_r.status_code == rs_r.status_code == 404)
    body_ok, body_msg = bodies_match(py_r.json(), rs_r.json(), "unknown_tool")
    results.record("REST (c) unknown_tool → 404 + body match", ok and body_ok, body_msg)

    py_al = read_last_audit_line(py_audit)
    rs_al = read_last_audit_line(rs_audit)
    audit_ok, audit_msg = audits_match(py_al, rs_al, "unknown_tool audit")
    results.record("REST (c) unknown_tool → audit match", audit_ok, audit_msg)

    # ── (d) not_bound: parity-self token, bot_guid != 777 ────────────────────

    py_r = hpost(f"{py_url}/v1/tools/bot.set_goal",
                 json_body={"bot_guid": 999, "goal": {}}, headers=auth_self)
    rs_r = hpost(f"{rs_url}/v1/tools/bot.set_goal",
                 json_body={"bot_guid": 999, "goal": {}}, headers=auth_self)
    ok = (py_r.status_code == rs_r.status_code == 403)
    py_body, rs_body = py_r.json(), rs_r.json()
    body_ok, body_msg = bodies_match(py_body, rs_body, "not_bound")
    err_ok = (py_body.get("error") == rs_body.get("error") == "not_bound")
    results.record("REST (d) not_bound → 403 + error=not_bound match",
                   ok and body_ok and err_ok, body_msg)

    py_al = read_last_audit_line(py_audit)
    rs_al = read_last_audit_line(rs_audit)
    audit_ok, audit_msg = audits_match(py_al, rs_al, "not_bound audit")
    results.record("REST (d) not_bound → audit match", audit_ok, audit_msg)

    # ── (e) gm.run_console allowlist reject ───────────────────────────────────

    py_r = hpost(f"{py_url}/v1/tools/gm.run_console",
                 json_body={"command": ".server info"}, headers=auth_all)
    rs_r = hpost(f"{rs_url}/v1/tools/gm.run_console",
                 json_body={"command": ".server info"}, headers=auth_all)
    ok = (py_r.status_code == rs_r.status_code == 403)
    body_ok, body_msg = bodies_match(py_r.json(), rs_r.json(), "run_console allowlist")
    results.record("REST (e) run_console allowlist → 403 match", ok and body_ok, body_msg)

    py_al = read_last_audit_line(py_audit)
    rs_al = read_last_audit_line(rs_audit)
    audit_ok, audit_msg = audits_match(py_al, rs_al, "run_console allowlist audit")
    results.record("REST (e) run_console allowlist → audit match", audit_ok, audit_msg)

    # ── (f) bad JSON body → 400 ──────────────────────────────────────────────

    py_r = hpost(f"{py_url}/v1/tools/obs.ping", data=b"{", headers=auth_all)
    rs_r = hpost(f"{rs_url}/v1/tools/obs.ping", data=b"{", headers=auth_all)
    ok = (py_r.status_code == rs_r.status_code == 400)
    py_body, rs_body = py_r.json(), rs_r.json()
    body_ok = (py_body.get("ok") == rs_body.get("ok") == False and
               py_body.get("error") == rs_body.get("error") == "bad_request")
    results.record("REST (f) bad JSON → 400 bad_request match", ok and body_ok,
                   f"py={py_body} rs={rs_body}" if not body_ok else "")

    # ── (g) unauth (no bearer) → 401 ─────────────────────────────────────────

    py_r = hpost(f"{py_url}/v1/tools/obs.ping", json_body={}, headers=no_auth)
    rs_r = hpost(f"{rs_url}/v1/tools/obs.ping", json_body={}, headers=auth_all)
    # unauth: only py should get 401, we want both with NO auth
    py_r = hpost(f"{py_url}/v1/tools/obs.ping", json_body={}, headers=no_auth)
    rs_r = hpost(f"{rs_url}/v1/tools/obs.ping", json_body={}, headers=no_auth)
    ok = (py_r.status_code == rs_r.status_code == 401)
    py_body, rs_body = py_r.json(), rs_r.json()
    err_match = (py_body.get("error") == rs_body.get("error"))
    results.record("REST (g) unauth → 401 + error match", ok and err_match,
                   f"py_error={py_body.get('error')!r} rs_error={rs_body.get('error')!r}"
                   if not err_match else "")

    # ── (h) ac_unreachable → 503 + Retry-After ───────────────────────────────

    print("    [stopping mock AC for unreachable test]")
    mock_ac.stop()
    time.sleep(0.4)

    py_r = hpost(f"{py_url}/v1/tools/obs.ping", json_body={}, headers=auth_all, timeout=20.0)
    rs_r = hpost(f"{rs_url}/v1/tools/obs.ping", json_body={}, headers=auth_all, timeout=20.0)

    ok = (py_r.status_code == rs_r.status_code == 503)
    py_body, rs_body = py_r.json(), rs_r.json()
    err_match = (py_body.get("error") == rs_body.get("error") == "unavailable")
    ra_py = py_r.headers.get("Retry-After")
    ra_rs = rs_r.headers.get("Retry-After")
    ra_match = (ra_py is not None and ra_rs is not None)
    results.record(
        "REST (h) ac_unreachable → 503 + Retry-After match",
        ok and err_match and ra_match,
        (f"statuses: py={py_r.status_code} rs={rs_r.status_code} | "
         f"errors: py={py_body.get('error')!r} rs={rs_body.get('error')!r} | "
         f"Retry-After: py={ra_py!r} rs={ra_rs!r}") if not (ok and err_match and ra_match) else "",
    )

    py_al = read_last_audit_line(py_audit)
    rs_al = read_last_audit_line(rs_audit)
    audit_ok, audit_msg = audits_match(py_al, rs_al, "ac_unreachable audit")
    results.record("REST (h) ac_unreachable → audit match", audit_ok, audit_msg)

    # Restart a fresh mock so health calls work
    print("    [restarting mock AC]")
    sys.path.insert(0, str(SCRIPT_DIR))
    from mock_ac import start_mock_ac
    new_mock = start_mock_ac()

    # ── (i) GET /v1/health ────────────────────────────────────────────────────

    py_r = hget(f"{py_url}/v1/health")
    rs_r = hget(f"{rs_url}/v1/health")
    ok = (py_r.status_code == rs_r.status_code == 200)
    py_body, rs_body = py_r.json(), rs_r.json()
    ok_field = (py_body.get("ok") == rs_body.get("ok") == True)
    ac_field = ("ac_bridge_reachable" in py_body and "ac_bridge_reachable" in rs_body)
    results.record("REST (i) GET /v1/health → 200 + shape match", ok and ok_field and ac_field,
                   f"py={py_body} rs={rs_body}" if not (ok and ok_field and ac_field) else "")

    # ── (j) GET /v1/audit?since=0 ────────────────────────────────────────────

    py_r = hget(f"{py_url}/v1/audit?since=0")
    rs_r = hget(f"{rs_url}/v1/audit?since=0")
    ok = (py_r.status_code == rs_r.status_code == 200)
    py_events = py_r.json().get("events", [])
    rs_events = rs_r.json().get("events", [])
    events_match = isinstance(py_events, list) and isinstance(rs_events, list)
    count_match = len(py_events) == len(rs_events)
    results.record(
        "REST (j) GET /v1/audit?since=0 → 200 + event count match",
        ok and events_match and count_match,
        f"py_count={len(py_events)} rs_count={len(rs_events)}" if not count_match else "",
    )

    return new_mock


# ── MCP battery ───────────────────────────────────────────────────────────────

async def run_mcp_battery(
    results: Results,
    py_url: str,
    rs_url: str,
    py_audit: str,
    rs_audit: str,
) -> None:
    print("\n--- MCP battery ---")
    if not MCP_AVAILABLE:
        results.record("MCP battery", False, "mcp package not available — SKIPPED")
        return

    py_mcp = f"{py_url}/mcp/mcp"
    rs_mcp = f"{rs_url}/mcp/mcp"
    bearer_all = {"Authorization": "Bearer parity-all"}
    bearer_obs = {"Authorization": "Bearer parity-obs"}

    # ── initialize ────────────────────────────────────────────────────────────

    async def do_initialize(mcp_url: str, headers: dict) -> dict:
        try:
            async with streamablehttp_client(mcp_url, headers=headers) as (r, w, _):
                async with ClientSession(r, w) as session:
                    result = await session.initialize()
                    return {
                        "protocol_version": result.protocolVersion,
                        "server_name":      result.serverInfo.name,
                        "server_version":   result.serverInfo.version,
                    }
        except Exception as e:
            return {"error": str(e)}

    py_init = await do_initialize(py_mcp, bearer_all)
    rs_init = await do_initialize(rs_mcp, bearer_all)

    protocol_ok = (py_init.get("protocol_version") == rs_init.get("protocol_version"))
    name_ok = (py_init.get("server_name") == rs_init.get("server_name") == "tot-harness")
    version_match = (py_init.get("server_version") == rs_init.get("server_version"))

    results.record("MCP initialize → protocolVersion matches", protocol_ok,
                   f"py={py_init.get('protocol_version')!r} rs={rs_init.get('protocol_version')!r}"
                   if not protocol_ok else "")
    results.record("MCP initialize → serverInfo.name='tot-harness' on both", name_ok,
                   f"py={py_init.get('server_name')!r} rs={rs_init.get('server_name')!r}"
                   if not name_ok else "")

    if not version_match:
        print(f"  [INFO] serverInfo.version differs: py={py_init.get('server_version')!r} "
              f"rs={rs_init.get('server_version')!r} — accepted if both non-empty")
    results.record("MCP initialize → serverInfo.version present on both",
                   bool(py_init.get("server_version") and rs_init.get("server_version")),
                   f"py={py_init.get('server_version')!r} rs={rs_init.get('server_version')!r}")

    # ── tools/list: 46 tools + description equality ───────────────────────────

    # ── Import brain's unwrap_fastmcp_args for envelope assertions ───────────
    import sys as _sys
    _brain_path = "/Users/tbrack/Documents/Projects/threads-of-time/tot/brain/brain_sidecar"
    if _brain_path not in _sys.path:
        _sys.path.insert(0, _brain_path)
    try:
        from schema_builder import unwrap_fastmcp_args as _unwrap_fastmcp_args
        _unwrap_available = True
    except ImportError as _e:
        _unwrap_available = False
        print(f"  [WARNING] could not import unwrap_fastmcp_args: {_e}", file=sys.stderr)

    async def list_tools_full(mcp_url: str, headers: dict) -> dict[str, Any]:
        """Returns {name: {'desc': str, 'schema': dict}} for all listed tools."""
        try:
            async with streamablehttp_client(mcp_url, headers=headers) as (r, w, _):
                async with ClientSession(r, w) as session:
                    await session.initialize()
                    result = await session.list_tools()
                    return {
                        t.name: {
                            "desc": t.description or "",
                            "schema": t.inputSchema if hasattr(t, "inputSchema") else {},
                        }
                        for t in result.tools
                    }
        except Exception as e:
            return {"_error": str(e)}

    py_tools_full = await list_tools_full(py_mcp, bearer_all)
    rs_tools_full = await list_tools_full(rs_mcp, bearer_all)

    py_tools = {n: v["desc"] for n, v in py_tools_full.items() if n != "_error"}
    rs_tools = {n: v["desc"] for n, v in rs_tools_full.items() if n != "_error"}
    py_schemas = {n: v["schema"] for n, v in py_tools_full.items() if n != "_error"}
    rs_schemas = {n: v["schema"] for n, v in rs_tools_full.items() if n != "_error"}

    count_ok = (len(py_tools) == len(rs_tools) == 46)
    results.record("MCP tools/list → 46 tools on both", count_ok,
                   f"py={len(py_tools)} rs={len(rs_tools)}")

    desc_ok = (py_tools == rs_tools)
    if not desc_ok:
        missing_in_rs = set(py_tools) - set(rs_tools)
        extra_in_rs   = set(rs_tools) - set(py_tools)
        desc_diffs = []
        for name in sorted(set(py_tools) & set(rs_tools)):
            if py_tools[name] != rs_tools[name]:
                desc_diffs.append(f"  {name}:\n    py={py_tools[name]!r}\n    rs={rs_tools[name]!r}")
        msg_parts = []
        if missing_in_rs: msg_parts.append(f"missing in rust: {sorted(missing_in_rs)}")
        if extra_in_rs:   msg_parts.append(f"extra in rust: {sorted(extra_in_rs)}")
        if desc_diffs:    msg_parts.append("description mismatches:\n" + "\n".join(desc_diffs))
        results.record("MCP tools/list → full {name:desc} equality", False, "\n".join(msg_parts))
    else:
        results.record("MCP tools/list → full {name:desc} equality", True)

    # ── HARD FAIL: every tool's inputSchema must have properties.args ─────────
    # The brain client (schema_builder.py:73 unwrap_fastmcp_args) RAISES
    # ValueError if properties.args is absent.  A daemon emitting flat schemas
    # (no args envelope) causes the brain to fail to boot.

    def check_args_envelope(schemas_dict: dict, label: str) -> tuple[bool, str]:
        """Assert every tool schema has properties.args; return (ok, msg)."""
        missing = []
        for name, schema in sorted(schemas_dict.items()):
            if not isinstance(schema, dict):
                missing.append(f"  {name}: schema is not a dict")
                continue
            props = schema.get("properties", {})
            if "args" not in props:
                missing.append(
                    f"  {name}: properties.args MISSING (keys={sorted(props.keys())})"
                )
        if missing:
            return False, f"{label} — {len(missing)} tools lack properties.args:\n" + "\n".join(missing)
        return True, ""

    py_env_ok, py_env_msg = check_args_envelope(py_schemas, "Python daemon")
    rs_env_ok, rs_env_msg = check_args_envelope(rs_schemas, "Rust daemon")
    results.record("MCP tools/list → Python daemon ALL tools have properties.args (brain contract)",
                   py_env_ok, py_env_msg)
    results.record("MCP tools/list → Rust daemon ALL tools have properties.args (brain contract)",
                   rs_env_ok, rs_env_msg)

    # ── brain's unwrap_fastmcp_args must not raise on any tool ────────────────
    if _unwrap_available:
        py_unwrap_failures = []
        rs_unwrap_failures = []
        for name, schema in sorted(py_schemas.items()):
            try:
                _unwrap_fastmcp_args(schema)
            except Exception as exc:
                py_unwrap_failures.append(f"  {name}: {exc}")
        for name, schema in sorted(rs_schemas.items()):
            try:
                _unwrap_fastmcp_args(schema)
            except Exception as exc:
                rs_unwrap_failures.append(f"  {name}: {exc}")
        results.record(
            "MCP tools/list → Python daemon schemas pass brain unwrap_fastmcp_args",
            len(py_unwrap_failures) == 0, "\n".join(py_unwrap_failures)
        )
        results.record(
            "MCP tools/list → Rust daemon schemas pass brain unwrap_fastmcp_args",
            len(rs_unwrap_failures) == 0, "\n".join(rs_unwrap_failures)
        )
    else:
        print("  [SKIP] brain unwrap_fastmcp_args check skipped — module not importable", file=sys.stderr)

    # ── Schema semantic comparison ────────────────────────────────────────────

    def resolve_refs(schema: dict, defs: dict) -> dict:
        if "$ref" in schema:
            ref = schema["$ref"]
            key = ref.split("/")[-1]
            resolved = defs.get(key, {})
            return resolve_refs(resolved, defs)
        out = {}
        for k, v in schema.items():
            if isinstance(v, dict):
                out[k] = resolve_refs(v, defs)
            elif isinstance(v, list):
                out[k] = [resolve_refs(i, defs) if isinstance(i, dict) else i for i in v]
            else:
                out[k] = v
        return out

    def normalize_schema_for_compare(schema: Any) -> dict:
        """Unwrap properties.args envelope, resolve $refs, strip 'format' annotations."""
        if not isinstance(schema, dict):
            return {}
        defs = schema.get("$defs", schema.get("definitions", {}))
        # Unwrap the {args: ...} envelope — both daemons MUST have it at this point
        props = schema.get("properties", {})
        inner = props.get("args", schema)
        resolved = resolve_refs(inner, defs)

        def strip_format(obj: Any) -> Any:
            if isinstance(obj, dict):
                return {k: strip_format(v) for k, v in obj.items() if k != "format"}
            if isinstance(obj, list):
                return [strip_format(i) for i in obj]
            return obj

        return strip_format(resolved)

    schema_mismatches = []
    for name in sorted(set(py_schemas) & set(rs_schemas)):
        py_norm = normalize_schema_for_compare(py_schemas[name])
        rs_norm = normalize_schema_for_compare(rs_schemas[name])
        py_props = set(py_norm.get("properties", {}).keys())
        rs_props = set(rs_norm.get("properties", {}).keys())
        if py_props != rs_props:
            schema_mismatches.append(
                f"  {name}: field mismatch py={sorted(py_props)} rs={sorted(rs_props)}"
            )
            continue
        py_req = set(py_norm.get("required", []))
        rs_req = set(rs_norm.get("required", []))
        if py_req != rs_req:
            schema_mismatches.append(
                f"  {name}: required mismatch py={sorted(py_req)} rs={sorted(rs_req)}"
            )

    results.record("MCP tools/list → schema semantic parity (fields + required)",
                   len(schema_mismatches) == 0, "\n".join(schema_mismatches))

    # ── tools/call obs.ping ───────────────────────────────────────────────────

    async def call_tool_raw(mcp_url: str, headers: dict, tool_name: str, args: dict) -> dict:
        """Call a tool via MCP; args passed as-is to session.call_tool."""
        try:
            async with streamablehttp_client(mcp_url, headers=headers) as (r, w, _):
                async with ClientSession(r, w) as session:
                    await session.initialize()
                    result = await session.call_tool(tool_name, args)
                    content = result.content
                    text = content[0].text if content else ""
                    try:
                        parsed = json.loads(text)
                    except json.JSONDecodeError:
                        parsed = {"_raw": text}
                    sc = getattr(result, "structuredContent", None)
                    return {
                        "isError": result.isError,
                        "content_parsed": parsed,
                        "has_structured_content": sc is not None,
                    }
        except Exception as e:
            return {"error": str(e), "isError": True, "content_parsed": {}}

    # Both daemons accept the {"args": {...}} envelope — this is the brain's
    # real wire contract (mcp_clients.py:63 hardcodes arguments={"args": args}).
    # Driving flat {} to either daemon would be WRONG.
    py_ping = await call_tool_raw(py_mcp, bearer_all, "obs.ping", {"args": {}})
    rs_ping = await call_tool_raw(rs_mcp, bearer_all, "obs.ping", {"args": {}})

    ping_err_ok = (py_ping.get("isError") == rs_ping.get("isError") == False)
    py_content = py_ping.get("content_parsed", {})
    rs_content = rs_ping.get("content_parsed", {})
    # Both must have the same error/ok field — either both ok=true (AC alive) or
    # both unavailable (AC unreachable at this point in the test run).
    # We assert isError=false on both (MCP-level success) and that both return
    # the SAME ok field value and the SAME error field value.
    contents_match = (py_content.get("ok") == rs_content.get("ok") and
                      py_content.get("error") == rs_content.get("error"))
    results.record("MCP tools/call obs.ping → isError=false + responses match on both",
                   ping_err_ok and contents_match,
                   f"py={py_ping} rs={rs_ping}" if not (ping_err_ok and contents_match) else "")

    # ── structuredContent (deferred item) ─────────────────────────────────────

    py_sc = py_ping.get("has_structured_content", False)
    rs_sc = rs_ping.get("has_structured_content", False)
    sc_match = (py_sc == rs_sc)
    print(f"  [INFO] structuredContent: py={py_sc} rs={rs_sc} — {'match' if sc_match else 'differ'}")
    results.record("MCP structuredContent → both match (present or absent)", sc_match,
                   f"py={py_sc} rs={rs_sc}")

    # ── scope_denied via MCP ──────────────────────────────────────────────────

    # Both daemons use the {"args": {...}} envelope.  Scope is denied before
    # args are deserialized, so the response is identical regardless.
    py_denied = await call_tool_raw(py_mcp, bearer_obs, "gm.teleport",
                                    {"args": {"target_guid": 1, "map": 0, "x": 0.0, "y": 0.0, "z": 0.0}})
    rs_denied = await call_tool_raw(rs_mcp, bearer_obs, "gm.teleport",
                                    {"args": {"target_guid": 1, "map": 0, "x": 0.0, "y": 0.0, "z": 0.0}})
    py_dc = py_denied.get("content_parsed", {})
    rs_dc = rs_denied.get("content_parsed", {})
    denied_ok = (
        py_denied.get("isError") == rs_denied.get("isError") == False and
        py_dc.get("ok") == rs_dc.get("ok") == False and
        py_dc.get("error") == rs_dc.get("error") == "scope_denied"
    )
    results.record("MCP scope_denied → success envelope, ok=false, error=scope_denied matches",
                   denied_ok, f"py={py_denied} rs={rs_denied}" if not denied_ok else "")

    # ── unauth MCP 401 (raw HTTP — check status + JSON body) ─────────────────
    # Python FastMCP emits JSON with spaces; Rust emits compact JSON.
    # Compare as parsed JSON objects (semantic equality, not byte equality).

    expected_401_json = {"error": "invalid_token", "error_description": "Authentication required"}

    async def raw_mcp_post(url: str) -> tuple[int, Any]:
        hdrs = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
        body = json.dumps({"jsonrpc": "2.0", "method": "initialize", "params": {}, "id": 1}).encode()
        async with httpx.AsyncClient(timeout=10.0) as client:
            r = await client.post(url, content=body, headers=hdrs)
            try:
                parsed = r.json()
            except Exception:
                parsed = {"_raw": r.text}
            return r.status_code, parsed

    py_status, py_401_json = await raw_mcp_post(py_mcp)
    rs_status, rs_401_json = await raw_mcp_post(rs_mcp)
    status_ok = (py_status == rs_status == 401)
    # Both must contain error=invalid_token and error_description=Authentication required
    py_ok = (py_401_json.get("error") == "invalid_token" and
             "Authentication" in str(py_401_json.get("error_description", "")))
    rs_ok = (rs_401_json.get("error") == "invalid_token" and
             "Authentication" in str(rs_401_json.get("error_description", "")))
    bodies_equal = (py_401_json == rs_401_json)
    results.record(
        "MCP unauth → 401 + body JSON-semantic match",
        status_ok and py_ok and rs_ok and bodies_equal,
        (f"py_status={py_status} rs_status={rs_status}\n"
         f"  py_body={py_401_json!r}\n"
         f"  rs_body={rs_401_json!r}") if not (status_ok and py_ok and rs_ok) else "",
    )
    if py_ok and rs_ok and not bodies_equal:
        print(f"  [INFO] MCP 401 bodies differ in whitespace/formatting but are semantically equal — "
              f"accepted deviation")

    # ── MCP-vs-REST args-shape (P4/A2): bot.set_strategy default fill ─────────

    auth_all_h = {"Authorization": "Bearer parity-all", "Content-Type": "application/json"}
    rest_args = {"bot_guid": 1, "strategy": "+follow"}  # omit bot_state → default "all"

    # REST calls
    hpost(f"{py_url}/v1/tools/bot.set_strategy", json_body=rest_args, headers=auth_all_h)
    hpost(f"{rs_url}/v1/tools/bot.set_strategy", json_body=rest_args, headers=auth_all_h)
    py_rest_audit = read_last_audit_line(py_audit)
    rs_rest_audit = read_last_audit_line(rs_audit)

    rest_hash_match = False
    rest_hash_msg = ""
    if py_rest_audit and rs_rest_audit:
        py_h = py_rest_audit.get("args_sha256", "")
        rs_h = rs_rest_audit.get("args_sha256", "")
        rest_hash_match = (py_h == rs_h)
        rest_hash_msg = f"py={py_h!r} rs={rs_h!r}" if not rest_hash_match else ""
    else:
        rest_hash_msg = f"missing audit lines: py={py_rest_audit} rs={rs_rest_audit}"
    results.record("MCP P4/A2 REST audit hashes equal (same raw args)", rest_hash_match, rest_hash_msg)

    # MCP calls — omit bot_state, should be filled to "all" by both.
    # Both daemons use the {"args": {...}} envelope (brain wire contract).
    py_mcp_strat = await call_tool_raw(py_mcp, bearer_all, "bot.set_strategy",
                                       {"args": {"bot_guid": 1, "strategy": "+follow"}})
    rs_mcp_strat = await call_tool_raw(rs_mcp, bearer_all, "bot.set_strategy",
                                       {"args": {"bot_guid": 1, "strategy": "+follow"}})
    py_mcp_audit = read_last_audit_line(py_audit)
    rs_mcp_audit = read_last_audit_line(rs_audit)

    mcp_hash_match = False
    mcp_hash_msg = ""
    if py_mcp_audit and rs_mcp_audit:
        py_h = py_mcp_audit.get("args_sha256", "")
        rs_h = rs_mcp_audit.get("args_sha256", "")
        mcp_hash_match = (py_h == rs_h)
        mcp_hash_msg = f"py={py_h!r} rs={rs_h!r}" if not mcp_hash_match else ""
        # Informational: what hash would "all"-filled args produce?
        filled_hash = sha256_args({"bot_guid": 1, "strategy": "+follow", "bot_state": "all"})
        print(f"  [INFO] MCP bot.set_strategy hash: py={py_h!r} (expected for 'all': {filled_hash!r})")
    else:
        mcp_hash_msg = f"missing audit lines: py={py_mcp_audit} rs={rs_mcp_audit}"
    results.record("MCP P4/A2 MCP audit hashes equal (default-fill parity)", mcp_hash_match, mcp_hash_msg)


# ── main ─────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description="Phase 12 parity gate")
    parser.add_argument("--rust-binary", default=str(RUST_BINARY_DEFAULT),
                        help="Path to the compiled harness-rs binary")
    parser.add_argument("--timeout", type=float, default=20.0,
                        help="Seconds to wait for each daemon to become healthy")
    args = parser.parse_args()

    rust_binary = Path(args.rust_binary)
    if not rust_binary.exists():
        print(f"ERROR: Rust binary not found at {rust_binary}", file=sys.stderr)
        sys.exit(1)
    if not PYTHON_DAEMON_BIN.exists():
        print(f"ERROR: Python daemon not found at {PYTHON_DAEMON_BIN}", file=sys.stderr)
        sys.exit(1)

    results = Results()

    # ── 1. Start mock AC ──────────────────────────────────────────────────────

    sys.path.insert(0, str(SCRIPT_DIR))
    from mock_ac import start_mock_ac

    print("Starting mock AC bridge...")
    mock = start_mock_ac()
    ac_url = mock.base_url
    print(f"  Mock AC at {ac_url}")

    # ── 2. Allocate ports + write fixtures ────────────────────────────────────

    py_port = free_port()
    rs_port = free_port()
    py_base = f"http://127.0.0.1:{py_port}"
    rs_base = f"http://127.0.0.1:{rs_port}"

    tmpdir = tempfile.mkdtemp(prefix="parity_")
    py_fixture = os.path.join(tmpdir, "py_tokens.yaml")
    rs_fixture = os.path.join(tmpdir, "rs_tokens.yaml")
    py_audit_f = os.path.join(tmpdir, "py_audit.jsonl")
    rs_audit_f = os.path.join(tmpdir, "rs_audit.jsonl")

    write_fixture(f"127.0.0.1:{py_port}", ac_url, py_audit_f, py_fixture)
    write_fixture(f"127.0.0.1:{rs_port}", ac_url, rs_audit_f, rs_fixture)

    print(f"Fixtures written to {tmpdir}")
    print(f"  Python  listen=127.0.0.1:{py_port}")
    print(f"  Rust    listen=127.0.0.1:{rs_port}")

    py_proc: Optional[subprocess.Popen] = None
    rs_proc: Optional[subprocess.Popen] = None

    try:
        # ── 3. Launch daemons ─────────────────────────────────────────────────

        print("\nStarting Python daemon (ORACLE)...")
        py_proc = start_python_daemon(py_fixture)
        try:
            wait_for_healthy(py_base, timeout=args.timeout, label="python")
            print(f"  Python healthy at {py_base}")
        except RuntimeError as e:
            try:
                py_proc.terminate()
                _, stderr_bytes = py_proc.communicate(timeout=5)
                print(f"  STDERR: {stderr_bytes.decode(errors='replace')[-1000:]}", file=sys.stderr)
            except Exception:
                pass
            print(f"FATAL: {e}", file=sys.stderr)
            mock.stop()
            sys.exit(1)

        print("Starting Rust daemon...")
        rs_proc = start_rust_daemon(rust_binary, rs_fixture)
        try:
            wait_for_healthy(rs_base, timeout=args.timeout, label="rust")
            print(f"  Rust healthy at {rs_base}")
        except RuntimeError as e:
            try:
                rs_proc.terminate()
                _, stderr_bytes = rs_proc.communicate(timeout=5)
                print(f"  STDERR: {stderr_bytes.decode(errors='replace')[-1000:]}", file=sys.stderr)
            except Exception:
                pass
            print(f"FATAL: {e}", file=sys.stderr)
            kill_proc(py_proc, "python")
            mock.stop()
            sys.exit(1)

        # ── 4. REST battery ───────────────────────────────────────────────────

        remaining_mock = run_rest_battery(
            results, py_base, rs_base, py_audit_f, rs_audit_f, mock
        )

        # ── 5. MCP battery ────────────────────────────────────────────────────

        asyncio.run(run_mcp_battery(
            results, py_base, rs_base, py_audit_f, rs_audit_f
        ))

        try:
            remaining_mock.stop()
        except Exception:
            pass

    finally:
        print("\nTearing down...")
        kill_proc(py_proc, "python")
        kill_proc(rs_proc, "rust")
        try:
            mock.stop()
        except Exception:
            pass

    all_ok = results.summary()
    sys.exit(0 if all_ok else 1)


if __name__ == "__main__":
    main()
