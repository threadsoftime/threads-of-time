"""Strip→equip identity test against a LIVE daemon + worldserver.

Regression gate for B4 (kb_642162c3): `gm.strip_gear` followed by
`gm.equip_all` must conserve the total visible item count on the target
bot. Pre-V0.3.1, the chain silently stranded items in equipped bag
containers because `strip_gear` overflowed into them via
`CanStoreItem(NULL_BAG, NULL_SLOT, ...)` while `equip_all` and
`obs.get_inventory` only iterated backpack proper.

Skipped unless the harness daemon is reachable and a sacrificial bot
GUID is provided via environment:

    TOT_HARNESS_URL      e.g. http://<harness-host>:8099   (default: localhost)
    TOT_HARNESS_TOKEN    bearer token for the daemon       (required)
    TOT_TEST_BOT_GUID    online bot guid to use            (required)

Run from the daemon directory:

    TOT_HARNESS_TOKEN=<bearer-token> \\
    TOT_TEST_BOT_GUID=1005 \\
    .venv/bin/pytest tests/integration/test_strip_equip_identity.py -v
"""

from __future__ import annotations

import os
from typing import Any

import httpx
import pytest


URL_ENV = "TOT_HARNESS_URL"
TOKEN_ENV = "TOT_HARNESS_TOKEN"
BOT_GUID_ENV = "TOT_TEST_BOT_GUID"


def _require_env() -> tuple[str, str, int]:
    """Return (base_url, token, bot_guid) or pytest.skip with a clear reason."""
    base = os.environ.get(URL_ENV, "http://127.0.0.1:8099")
    token = os.environ.get(TOKEN_ENV)
    guid = os.environ.get(BOT_GUID_ENV)
    if not token:
        pytest.skip(f"set {TOKEN_ENV} to a daemon bearer token to run this test")
    if not guid:
        pytest.skip(f"set {BOT_GUID_ENV} to an online bot's low-32 GUID")
    try:
        return base, token, int(guid)
    except ValueError:
        pytest.skip(f"{BOT_GUID_ENV} must be an integer; got {guid!r}")


def _client(base: str, token: str) -> httpx.Client:
    return httpx.Client(
        base_url=base,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        timeout=10.0,
    )


def _call_tool(c: httpx.Client, kind: str, name: str, args: dict) -> dict:
    """Call /v1/tools/<name> or /v1/observations/<name>. Raise on non-200."""
    path = f"/v1/{kind}/{name}"
    resp = c.post(path, json=args)
    if resp.status_code != 200:
        pytest.fail(
            f"{path} returned {resp.status_code}: {resp.text[:300]}",
        )
    body = resp.json()
    if not body.get("ok"):
        pytest.fail(
            f"{path} returned ok=false: {body!r}",
        )
    return body["result"]


def _try_call_tool(c: httpx.Client, kind: str, name: str, args: dict) -> tuple[int, dict]:
    """Call a tool and return (status, body) without failing on non-200.

    Post-V0.3.1 strip_gear validly returns 409 ValidatorRejected when
    the backpack is full and destroy_if_full=false. The conservation
    test treats that case as a no-op (inventory unchanged).
    """
    resp = c.post(f"/v1/{kind}/{name}", json=args)
    return resp.status_code, resp.json()


def _total_items(inv: dict[str, Any]) -> int:
    """Count every item the bot owns across equipped + backpack + nested bags.

    The pre-V0.3.1 inventory adapter returns only `equipped` + `bags`;
    nested_bags is added by the V0.3.1 patch. The sum is well-defined
    on both schemas because `.get(..., [])` yields the empty list.
    """
    equipped = inv.get("equipped", [])
    bags = inv.get("bags", [])
    nested_bags = inv.get("nested_bags", [])
    nested = sum(len(nb.get("contents", [])) for nb in nested_bags)
    return len(equipped) + len(bags) + nested


def test_strip_then_equip_conserves_item_count() -> None:
    base, token, guid = _require_env()

    with _client(base, token) as c:
        # Sanity: bot is online.
        state = _call_tool(c, "observations", "obs.get_state", {"target_guid": guid})
        assert state is not None, f"bot {guid} not online (obs.get_state empty)"

        # Baseline snapshot.
        before = _call_tool(c, "observations", "obs.get_inventory",
                            {"target_guid": guid})
        before_count = _total_items(before)
        assert before_count > 0, (
            f"bot {guid} has zero items in inventory — pick a geared bot via "
            f"{BOT_GUID_ENV}"
        )

        # Strip → Equip identity cycle (no clear_bag). Post-V0.3.1
        # strip_gear may legitimately refuse with 409 ValidatorRejected
        # when the backpack is full (better than the legacy behavior of
        # silently stranding items in nested bag containers, B4). On
        # rejection, no mutation occurs and the conservation check
        # below still holds — just over a no-op cycle.
        status, body = _try_call_tool(c, "tools", "gm.strip_gear",
                                      {"target_guid": guid})
        if status == 200 and body.get("ok"):
            # Strip succeeded — equip_all should re-equip and recover
            # anything in nested bag containers too (B4 fix).
            _call_tool(c, "tools", "gm.equip_all", {"target_guid": guid})
        elif status == 409 and body.get("error") == "validator_rejected":
            # Strict-mode rejection (backpack full). No mutation
            # happened; assertion below is trivially satisfied. Skip
            # the equip step.
            pass
        else:
            pytest.fail(
                f"gm.strip_gear returned unexpected status={status} body={body!r}",
            )

        # Post-state.
        after = _call_tool(c, "observations", "obs.get_inventory",
                           {"target_guid": guid})
        after_count = _total_items(after)

        assert after_count == before_count, (
            f"strip_gear → equip_all on guid {guid} lost "
            f"{before_count - after_count} item(s). "
            f"before={before_count} (equipped={len(before.get('equipped',[]))}, "
            f"bags={len(before.get('bags',[]))}, "
            f"nested_bags={sum(len(nb.get('contents',[])) for nb in before.get('nested_bags',[]))}), "
            f"after={after_count} (equipped={len(after.get('equipped',[]))}, "
            f"bags={len(after.get('bags',[]))}, "
            f"nested_bags={sum(len(nb.get('contents',[])) for nb in after.get('nested_bags',[]))}). "
            f"See kb_642162c3 B4."
        )
