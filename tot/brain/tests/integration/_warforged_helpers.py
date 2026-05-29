"""Helpers for mod-warforged live-server integration tests.

These tests are marked ``@pytest.mark.live_server`` and ``@pytest.mark.warforged``
and are deselected by default (see ``pyproject.toml``). They are designed to
run on the deploy host after Task 24 (M9 image bake) lands. Until then they
exercise the test scaffolding only; the assertions speak to behavior of the
deployed worldserver + harness daemon.

Two surfaces are wrapped here:

1. A minimal HTTP client mirroring ``tools/smoke-tests/conftest.py:Harness`` so
   these tests don't need the smoke-tests package on PYTHONPATH. We talk to the
   daemon's REST surface (``/v1/tools/*``, ``/v1/observations/*``).

2. A pair of convenience constants — Bracket-1 Epic enchant 70003 (+2 STR / +2
   STA / +2 CRIT_RATING) — pulled from
   ``modules/mod-warforged/data/csv/warforged_enchants.csv``.

The tests skip cleanly when ``HARNESS_URL`` / ``HARNESS_SMOKE_TOKEN`` are not
set so CI / `pytest -m 'not live_server'` runs are unaffected.
"""
from __future__ import annotations

import os
from typing import Any

import httpx
import pytest

# -----------------------------------------------------------------------------
# Warforged constants — match modules/mod-warforged/src/WarforgedConstants.h
# -----------------------------------------------------------------------------

WF_ENCHANT_MIN: int = 70001
WF_ENCHANT_MAX: int = 70063  # 9 bands x 7 enchant_ids per band, sparse range

# Bracket-1 Epic (band 1-25 quality 4). From warforged_enchants.csv line 26.
# Enchant 70003: +2 STR (4), +2 STA (7), +2 CRIT_RATING (32). Stat 1 is the
# "primary stat" placeholder per spec §12 (always STR in v1.0).
WF_BRACKET1_EPIC_ENCHANT_ID: int = 70003
WF_BRACKET1_EPIC_STR_DELTA: int = 2
WF_BRACKET1_EPIC_STA_DELTA: int = 2
WF_BRACKET1_EPIC_CRIT_DELTA: int = 2

# Item enchantment slot indices (Item.h: EnchantmentSlot)
WF_BONUS_SLOT: int = 5  # PROP_ENCHANTMENT_SLOT_0 — repurposed by mod-warforged
WF_SOCKET_SLOT: int = 6  # PROP_ENCHANTMENT_SLOT_1 — PRISMATIC, value 3729

WF_PRISMATIC_SPELL_ID: int = 3729


# -----------------------------------------------------------------------------
# HTTP harness client (subset of tools/smoke-tests/conftest.py:Harness)
# -----------------------------------------------------------------------------


class HarnessClient:
    """Subset of the daemon HTTP surface needed by these tests.

    Kept self-contained (no dependency on tools/smoke-tests) so this test
    file can run from the brain-sidecar package alone.
    """

    def __init__(self, url: str, token: str) -> None:
        self._client = httpx.Client(
            base_url=url,
            timeout=30.0,
            headers={
                "Authorization": f"Bearer {token}",
                "Content-Type": "application/json",
            },
        )

    def close(self) -> None:
        self._client.close()

    # --- query_db ------------------------------------------------------------

    def query_db(self, template_name: str, **params: Any) -> list[dict]:
        r = self._client.post(
            "/v1/observations/obs.query_db",
            json={"template_name": template_name, "params": params},
        )
        r.raise_for_status()
        return r.json()["result"]["rows"]

    # --- gm.* ----------------------------------------------------------------

    def run_console_and_wait(
        self, command: str, *, timeout_s: float = 10.0
    ) -> dict:
        """Submit a console command, poll until done, return final result."""
        import time as _t

        r = self._client.post(
            "/v1/tools/gm.run_console", json={"command": command}
        )
        r.raise_for_status()
        request_id = r.json()["result"]["request_id"]

        deadline = _t.monotonic() + timeout_s
        backoff_ms = 100
        while _t.monotonic() < deadline:
            r = self._client.post(
                "/v1/tools/gm.read_console_output",
                json={"request_id": request_id},
            )
            r.raise_for_status()
            result = r.json()["result"]
            if result.get("found") and result.get("done"):
                return result
            _t.sleep(backoff_ms / 1000.0)
            backoff_ms = min(backoff_ms * 2, 1000)
        raise TimeoutError(
            f"run_console did not complete within {timeout_s}s: {command!r}"
        )

    def additem(self, target_guid: int, item_entry: int, count: int = 1) -> int:
        r = self._client.post(
            "/v1/tools/gm.additem",
            json={"target_guid": target_guid, "item_entry": item_entry, "count": count},
        )
        r.raise_for_status()
        return r.json()["result"]["added"]

    def equip_all(self, target_guid: int) -> list[int]:
        r = self._client.post(
            "/v1/tools/gm.equip_all", json={"target_guid": target_guid}
        )
        r.raise_for_status()
        return r.json()["result"]["equipped_slots"]

    def strip_gear(
        self,
        target_guid: int,
        destroy_if_full: bool = False,
        clear_bag: bool = False,
    ) -> dict:
        r = self._client.post(
            "/v1/tools/gm.strip_gear",
            json={
                "target_guid": target_guid,
                "destroy_if_full": destroy_if_full,
                "clear_bag": clear_bag,
            },
        )
        r.raise_for_status()
        return r.json()["result"]

    # --- obs.* ---------------------------------------------------------------

    def get_state(self, target_guid: int) -> dict:
        r = self._client.post(
            "/v1/observations/obs.get_state", json={"target_guid": target_guid}
        )
        r.raise_for_status()
        return r.json()["result"]

    def get_inventory(self, target_guid: int) -> dict:
        r = self._client.post(
            "/v1/observations/obs.get_inventory",
            json={"target_guid": target_guid},
        )
        r.raise_for_status()
        return r.json()["result"]


# -----------------------------------------------------------------------------
# Pytest fixtures
# -----------------------------------------------------------------------------


def _harness_config() -> tuple[str, str] | None:
    url = os.environ.get("HARNESS_URL")
    token = os.environ.get("HARNESS_SMOKE_TOKEN")
    if not url or not token:
        return None
    return (url, token)


@pytest.fixture(scope="module")
def harness() -> HarnessClient:
    """Live harness client. Skips module if env vars are not set."""
    cfg = _harness_config()
    if cfg is None:
        pytest.skip(
            "HARNESS_URL / HARNESS_SMOKE_TOKEN not set — Warforged live tests "
            "require a deployed ToT worldserver (see Task 24)."
        )
    url, token = cfg
    h = HarnessClient(url, token)
    yield h
    h.close()


# -----------------------------------------------------------------------------
# Shared helpers
# -----------------------------------------------------------------------------


def parse_enchant_slots(enchantments_blob: str) -> list[int]:
    """Parse the 33-space-separated-ints ``enchantments`` column of
    ``item_instance``. Returns the list of slot[i].id values (index 0 = perm,
    5 = BONUS/WARFORGED, 6 = PRISMATIC).

    The column is ``"id duration charges id duration charges ..."`` with 11
    slot triples. See ``Item::LoadFromDB`` in azerothcore for the canonical
    parser.
    """
    parts = enchantments_blob.split()
    return [int(parts[i * 3]) for i in range(11) if i * 3 < len(parts)]


def is_warforged_enchant(enchant_id: int) -> bool:
    """Match ``IsWarforgedEnchant`` in
    ``modules/mod-warforged/src/WarforgedConstants.h``.
    """
    return WF_ENCHANT_MIN <= enchant_id <= WF_ENCHANT_MAX
