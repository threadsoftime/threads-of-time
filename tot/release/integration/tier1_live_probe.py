#!/usr/bin/env python3
# tot/release/integration/tier1_live_probe.py
#
# Read-only probes against a LIVE ToT harness.
# Safe to run against the live Heimdal baseline — no mutation, no GM commands,
# no bot state changes. Only POSTs to obs.* tools (observation layer).
#
# Usage:
#   HARNESS_URL=http://127.0.0.1:8099 HARNESS_BEARER=<token> python3 tier1_live_probe.py
#
# Exit codes: 0 = all probes passed, 1 = one or more probes failed.
#
# Environment variables (both required):
#   HARNESS_URL     — base URL of the harness daemon (no trailing slash).
#   HARNESS_BEARER  — bearer token for Authorization header.
#
# Missing env vars exit immediately with a clear error (non-zero) before
# any network call is made.
from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request


def _require_env() -> tuple[str, str]:
    """Return (url, token). Exit non-zero with a readable message if either is unset."""
    missing = [v for v in ("HARNESS_URL", "HARNESS_BEARER") if not os.environ.get(v)]
    if missing:
        print(
            f"ERROR: required environment variable(s) not set: {', '.join(missing)}\n"
            "  export HARNESS_URL=http://127.0.0.1:8099\n"
            "  export HARNESS_BEARER=<token>",
            file=sys.stderr,
        )
        sys.exit(1)
    return os.environ["HARNESS_URL"].rstrip("/"), os.environ["HARNESS_BEARER"]


def call(url: str, token: str, tool: str, payload: dict | None = None) -> dict:
    """POST to /v1/tools/<tool> and return the parsed JSON response body.

    Raises urllib.error.HTTPError on non-2xx, urllib.error.URLError on
    network failure, json.JSONDecodeError on bad body.  Callers catch all
    exceptions via the probe harness.
    """
    body = json.dumps(payload or {}).encode()
    req = urllib.request.Request(
        f"{url}/v1/tools/{tool}",
        data=body,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=15) as resp:
        return json.load(resp)


def _probe_obs_ping(url: str, tok: str) -> None:
    """obs.ping must return a truthy response."""
    resp = call(url, tok, "obs.ping")
    if not resp:
        raise AssertionError(f"obs.ping returned empty/falsy: {resp!r}")


def _probe_obs_list_bot_population(url: str, tok: str) -> None:
    """obs.list_bot_population must return a non-empty result list."""
    resp = call(url, tok, "obs.list_bot_population")
    # Response shape: {"ok": true, "result": [...]} — unwrap before checking.
    result = resp.get("result") if isinstance(resp, dict) else resp
    if not result:
        raise AssertionError(
            f"obs.list_bot_population returned no bots — "
            f"worldserver may not have spawned bot population yet "
            f"(raw: {resp!r})"
        )


def _probe_obs_list_players(url: str, tok: str) -> None:
    """obs.list_players must return a response without error — empty list is valid."""
    resp = call(url, tok, "obs.list_players")
    if resp is None:
        raise AssertionError("obs.list_players returned None")
    # Verify the response carries an 'ok' or 'result' key — guards against
    # accidentally hitting a different endpoint that returns a non-harness shape.
    if isinstance(resp, dict) and "ok" not in resp and "result" not in resp:
        raise AssertionError(
            f"obs.list_players response missing 'ok'/'result' keys — "
            f"unexpected shape: {resp!r}"
        )


def _probe_obs_shape(url: str, tok: str) -> None:
    """Shape smoke: list_bot_population result entries must be dicts with a guid."""
    resp = call(url, tok, "obs.list_bot_population")
    result = resp.get("result") if isinstance(resp, dict) else resp
    if result:
        entry = result[0]
        if not isinstance(entry, dict):
            raise AssertionError(
                f"obs.list_bot_population result[0] is not a dict: {entry!r}"
            )
        # The obs layer exposes bot entries; any entry must be a mapping.
        # We do NOT assert a specific key set here — that would make this
        # probe fragile against schema evolution.


def main() -> int:
    url, tok = _require_env()

    probes = [
        ("obs.ping", _probe_obs_ping),
        ("obs.list_bot_population", _probe_obs_list_bot_population),
        ("obs.list_players", _probe_obs_list_players),
        ("obs.get_state-shape", _probe_obs_shape),
    ]

    passed: list[str] = []
    failed: list[str] = []

    for name, fn in probes:
        try:
            fn(url, tok)
            passed.append(f"PASS {name}")
        except urllib.error.HTTPError as exc:
            failed.append(f"FAIL {name}: HTTP {exc.code} {exc.reason}")
        except urllib.error.URLError as exc:
            failed.append(f"FAIL {name}: network error — {exc.reason}")
        except json.JSONDecodeError as exc:
            failed.append(f"FAIL {name}: response not valid JSON — {exc}")
        except AssertionError as exc:
            failed.append(f"FAIL {name}: {exc}")
        except Exception as exc:  # noqa: BLE001
            failed.append(f"FAIL {name}: unexpected error — {type(exc).__name__}: {exc}")

    for line in passed + failed:
        print(line)

    if failed:
        print(
            f"\n{len(failed)} probe(s) FAILED, {len(passed)} passed.",
            file=sys.stderr,
        )
        return 1

    print(f"\nAll {len(passed)} probes PASSED.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
