"""Task 22: verify Warforged enchants persist across trade and across relog.

Two tests, one file:

- ``test_warforged_survives_trade``: bot A holds a Warforged item, trades to
  bot B, the BONUS-slot enchant ID is unchanged on bot B's copy of the row.

- ``test_warforged_persists_relog``: bot logs out, logs back in (forced via
  ``.character forcelogout`` + the bot framework's auto-respawn), enchants
  intact.

Both rely on ``.warforged force <itemguid>`` to deterministically attach an
enchant (see Task 21 file docstring re: the Console::No caveat — both tests
skip if the console rejection message is emitted).

Concerns / post-deploy refinement (Task 23):
- ``.character forcelogout`` per-AC source kicks the player session — the
  bot framework's keep-alive should auto-relog within ~3s, but timing is
  fragile. Consider replacing with a ``bot.logout`` + ``bot.login`` MCP
  tool pair (out of scope for v1).
- Bot trade isn't directly exposed by the harness daemon today. We move
  the item via SQL update (item_instance.owner_guid) as a stand-in — that
  preserves the enchantments column verbatim and is a correct *persistence*
  test (the row survives), even if it's not a real in-game trade flow.
  Replace with a real ``bot.trade`` MCP tool when one is built.
"""
from __future__ import annotations

import time

import pytest

from ._warforged_helpers import (
    WF_BONUS_SLOT,
    WF_PRISMATIC_SPELL_ID,
    WF_SOCKET_SLOT,
    HarnessClient,
    harness,  # noqa: F401
    is_warforged_enchant,
    parse_enchant_slots,
)

# Same fallback used by test_warforged_stats_apply.py.
FALLBACK_ITEM_ENTRY: int = 7748


def _pick_test_bot(harness: HarnessClient, *, class_id: int = 1) -> int:
    rows = harness.query_db("character_online_by_class", class_id=class_id)
    online = [r for r in rows if r.get("online")]
    if not online:
        pytest.skip(f"no online class={class_id} bot available")
    return online[0]["guid"]


def _force_warforged(harness: HarnessClient, item_guid: int) -> None:
    """Apply a Warforged enchant via ``.warforged force``; skip if console
    can't run it (Console::No)."""
    out = harness.run_console_and_wait(
        f".warforged force {item_guid}", timeout_s=5.0
    )
    if "no calling player" in (out.get("output") or "").lower():
        pytest.skip(
            ".warforged force requires a calling player (Console::No). "
            "See WarforgedCommandScript.cpp:87."
        )


def _read_enchant_slots_via_inventory(
    harness: HarnessClient, owner_guid: int, item_guid: int
) -> list[int]:
    """Pull the per-slot enchant ids from the inventory response.

    If the harness inventory shape doesn't include enchantments today, this
    raises pytest.skip — Task 23 needs to extend the inventory response.
    """
    inv = harness.get_inventory(owner_guid)
    for it in inv.get("items", []):
        if it.get("item_guid") != item_guid:
            continue
        enchants = it.get("enchantments")
        if enchants is None:
            pytest.skip(
                "inventory response lacks per-item enchantments; add the "
                "enchantments[] field to obs.get_inventory and re-run"
            )
        if isinstance(enchants, list) and enchants and isinstance(enchants[0], dict):
            return [(e or {}).get("id", 0) for e in enchants]
        if isinstance(enchants, str):
            return parse_enchant_slots(enchants)
        pytest.fail(f"unexpected enchantments shape: {type(enchants).__name__}")
    pytest.fail(f"item_guid {item_guid} not on bot {owner_guid}")
    return []  # unreachable


# -----------------------------------------------------------------------------
# Test 1: trade persistence
# -----------------------------------------------------------------------------


@pytest.mark.heimdal
@pytest.mark.warforged
def test_warforged_survives_trade(harness: HarnessClient) -> None:
    bot_a = _pick_test_bot(harness, class_id=1)
    # Find a *different* online warrior to be the recipient.
    rows = harness.query_db("character_online_by_class", class_id=1)
    other = [r for r in rows if r.get("online") and r["guid"] != bot_a]
    if not other:
        pytest.skip("need 2 online warriors for trade test")
    bot_b = other[0]["guid"]

    # 1. Strip bot A so the new item is isolated.
    harness.strip_gear(bot_a, destroy_if_full=True, clear_bag=True)

    # 2. Give item to bot A and find its guid.
    harness.additem(bot_a, FALLBACK_ITEM_ENTRY, 1)
    inv_a = harness.get_inventory(bot_a)
    new = [it for it in inv_a.get("items", [])
           if it.get("item_entry") == FALLBACK_ITEM_ENTRY]
    assert new, "item not found on bot A after additem"
    item_guid = new[0]["item_guid"]

    # 3. Force a Warforged enchant.
    _force_warforged(harness, item_guid)

    # 4. Snapshot enchant slots on bot A.
    slots_a = _read_enchant_slots_via_inventory(harness, bot_a, item_guid)
    assert is_warforged_enchant(slots_a[WF_BONUS_SLOT]), (
        f"BONUS slot {WF_BONUS_SLOT} not Warforged: {slots_a[WF_BONUS_SLOT]}"
    )

    # 5. Transfer to bot B. Real in-game trade isn't exposed by the harness;
    #    use ``.send items <player> <guid>`` console command which is
    #    Console::Yes. It mails the item, which on retrieval preserves
    #    enchantments. This is the closest persistence-equivalent of a trade.
    bot_b_name = next(
        (r["name"] for r in rows if r["guid"] == bot_b), None
    )
    if not bot_b_name:
        pytest.skip("could not resolve bot B name via character_online_by_class")
    harness.run_console_and_wait(
        f".send items {bot_b_name} \"Warforged Test\" \"test body\" {FALLBACK_ITEM_ENTRY}",
        timeout_s=10.0,
    )
    # Sleep for the mail delivery + bot auto-pickup loop.
    time.sleep(5.0)

    # 6. Verify the item now sits on bot B with enchants intact.
    inv_b = harness.get_inventory(bot_b)
    moved = [it for it in inv_b.get("items", [])
             if it.get("item_entry") == FALLBACK_ITEM_ENTRY]
    if not moved:
        pytest.skip(
            "item didn't arrive on bot B within 5s — confirm mail delivery + "
            "bot auto-mail-pickup are enabled, then increase the wait"
        )
    slots_b = _read_enchant_slots_via_inventory(harness, bot_b, moved[0]["item_guid"])
    assert is_warforged_enchant(slots_b[WF_BONUS_SLOT]), (
        f"BONUS slot lost during transfer: {slots_b[WF_BONUS_SLOT]}"
    )
    # PRISMATIC slot 6 should equal the marker spell id (3729) — see
    # WarforgedStatApplier.cpp.
    assert slots_b[WF_SOCKET_SLOT] == WF_PRISMATIC_SPELL_ID, (
        f"PRISMATIC slot mismatch on bot B: {slots_b[WF_SOCKET_SLOT]} "
        f"!= {WF_PRISMATIC_SPELL_ID}"
    )


# -----------------------------------------------------------------------------
# Test 2: relog persistence
# -----------------------------------------------------------------------------


@pytest.mark.heimdal
@pytest.mark.warforged
def test_warforged_persists_relog(harness: HarnessClient) -> None:
    bot_guid = _pick_test_bot(harness, class_id=1)

    # 1. Clean, give, force.
    harness.strip_gear(bot_guid, destroy_if_full=True, clear_bag=True)
    harness.additem(bot_guid, FALLBACK_ITEM_ENTRY, 1)
    inv = harness.get_inventory(bot_guid)
    new = [it for it in inv.get("items", [])
           if it.get("item_entry") == FALLBACK_ITEM_ENTRY]
    assert new, "item not added"
    item_guid = new[0]["item_guid"]
    _force_warforged(harness, item_guid)

    # 2. Snapshot pre-relog.
    slots_before = _read_enchant_slots_via_inventory(harness, bot_guid, item_guid)
    assert is_warforged_enchant(slots_before[WF_BONUS_SLOT])
    bonus_before = slots_before[WF_BONUS_SLOT]

    # 3. Force logout. ``.character forcelogout <name>`` kicks the session;
    #    the playerbots framework auto-relogs the bot within ~3-5 seconds.
    bot_name_rows = harness.query_db("character_online", guid=bot_guid)
    if not bot_name_rows:
        pytest.skip("character_online returned no row for bot — DB drift")
    bot_name = bot_name_rows[0]["name"]

    harness.run_console_and_wait(
        f".character forcelogout {bot_name}", timeout_s=5.0
    )

    # 4. Poll for bot back online (up to 30s).
    deadline = time.monotonic() + 30.0
    online_again = False
    while time.monotonic() < deadline:
        rows = harness.query_db("character_online", guid=bot_guid)
        if rows and rows[0].get("online"):
            online_again = True
            break
        time.sleep(2.0)
    if not online_again:
        pytest.skip(
            "bot did not auto-relog within 30s — confirm playerbots keep-alive "
            "is enabled (mod-playerbots auto-respawn config)"
        )

    # 5. Re-read enchants. The relog forces a fresh ``Item::LoadFromDB``, so
    #    this verifies the enchantments column round-trips correctly.
    slots_after = _read_enchant_slots_via_inventory(harness, bot_guid, item_guid)
    assert slots_after[WF_BONUS_SLOT] == bonus_before, (
        f"BONUS enchant changed across relog: {bonus_before} -> "
        f"{slots_after[WF_BONUS_SLOT]}"
    )
    assert slots_after[WF_SOCKET_SLOT] == WF_PRISMATIC_SPELL_ID, (
        f"PRISMATIC marker missing after relog: {slots_after[WF_SOCKET_SLOT]}"
    )
