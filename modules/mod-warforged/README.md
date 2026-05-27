# mod-warforged

Legion-style Warforged + bonus socket procs on item drops.

**Status:** v1.0 (development). See [design spec](../../docs/superpowers/specs/2026-05-23-mod-warforged-design.md).

## Slot ownership convention

This module owns two per-instance enchantment slots:
- `BONUS_ENCHANTMENT_SLOT` (slot 5) — Warforged stat bump enchant
- `PRISMATIC_ENCHANTMENT_SLOT` (slot 6) — bonus socket (NOT used on belts to avoid Eternal Belt Buckle collision)

`mod-bracket-sets` does not touch these slots (verified). New modules must not write to slot 5 or slot 6 without coordination.

## Build

Standard AzerothCore module. `cmake -j4` per project convention (kb_57b453cd).

## Config

See `conf/mod_warforged.conf.dist`.

## GM commands

- `.warforged info <itemguid>` — display BONUS and PRISMATIC slot contents
- `.warforged strip <itemguid>` — clear slots 5 and 6
- `.warforged force <itemguid>` — force a Warforged + socket roll (testing)

## Client patch

`client/patch-W.MPQ` must be installed by every player at `World of Warcraft/Data/`. Without it, Warforged stats apply correctly server-side but the tooltip shows a default green enchant line at the bottom instead of the orange "Warforged" tag under the item name.

### Installation

1. Download `client/patch-W.MPQ` from this module (or distribute via your launcher / forum / Discord pin).
2. Drop it in your WoW 3.3.5a client's `Data/` folder, alongside the stock `patch-enUS-4.MPQ` etc.
3. Restart the client.

### Verifying installation

In-game, type:

```
/script print(WARFORGED_PATCH_VERSION)
```

You should see `1.0.0` printed in chat. If you see `nil`, the patch isn't loaded — verify the MPQ is in `Data/` and the client was restarted after dropping it in.

The server also runs a sentinel handshake on login (see `WarforgedSentinelScript`). If `Warforged.WarnMissingPatch = 1` is set in `mod_warforged.conf`, players without the patch get a whisper at login telling them to install it.

### Without the patch — degraded rendering

Stats apply correctly server-side regardless of patch presence. The visual difference:

- **With patch:** the orange "Warforged" tag appears in the tooltip (currently appended at bottom; v1.1 will move it under the item name via `FontString` reordering). Item-level line shows base+5. Bonus socket (if rolled) appears as a normal-looking prismatic socket and is gemmable.
- **Without patch:** stats apply correctly but the tooltip shows the bump as a generic green enchant line at the bottom reading `Warforged: +X Strength, +Y Stamina`. The item-level line shows the base ilvl. Bonus socket still appears (the `PRISMATIC_ENCHANTMENT_SLOT` is rendered natively by the 3.3.5a client).

The game is fully playable either way — the patch is polish, not gameplay.

### Building the patch from source

```bash
# 1. Patch the DBC with Warforged enchant rows + emit Lua stat-bumps table
python3 modules/mod-warforged/tools/build-warforged-dbc.py

# 2. Pack DBC + Lua into the MPQ (StormLib via ctypes; brew install stormlib on macOS)
python3 modules/mod-warforged/tools/pack-mpq.py
```

Output lands at `modules/mod-warforged/client/patch-W.MPQ` (~52 KB). The binary is committed to the repo so distributors don't need a build toolchain.

## API notes

Verified against the `mod-playerbots/azerothcore-wotlk` fork at pinned SHA `f570462f` (see `image/build.sh:AC_BASE_SHA`). All line numbers below are anchors against that SHA; if upstream drifts, re-verify before editing the stat-applier.

### Enchant slot write sequence

At item-drop time, `WarforgedStatApplier` (Task 8) calls **only** `Item::SetEnchantment` on the freshly-created `Item*`. It does NOT call `Player::ApplyEnchantment` directly — AC's equip pipeline handles runtime stat application automatically.

The minimal write sequence is:

```cpp
// Stat bump (always, when the item rolls Warforged)
item->SetEnchantment(BONUS_ENCHANTMENT_SLOT, wfEnchantId, /*duration=*/0, /*charges=*/0);

// Optional bonus socket (not on belts — Eternal Belt Buckle owns PRISMATIC slot for belts)
item->SetEnchantment(PRISMATIC_ENCHANTMENT_SLOT, /*socket enchant id=*/3729, 0, 0);
```

`Item::SetEnchantment` (Item.cpp:920) sets `ITEM_FIELD_ENCHANTMENT_1_1 + slot*MAX_ENCHANTMENT_OFFSET + …` UpdateFields and marks the item `ITEM_CHANGED` (Item.cpp:936-939), which causes the next item-save tick to persist the enchant to `character_inventory.itemEntry` / the item's row in `item_instance` (enchant blob column). It does NOT touch player stats.

`Player::ApplyEnchantment(Item*, bool apply)` (PlayerStorage.cpp:4391) is the runtime stat modifier. It dispatches to the per-slot overload (PlayerStorage.cpp:4397), which short-circuits with `if (!item || !item->IsEquipped()) return;` at line 4399. **The bump is only visible on a player's character sheet while the item is in an equipment slot.**

AC drives this automatically. Two relevant entry points cover every realistic Warforged drop scenario:

1. **Player equips the item normally** (loot → bag → drag to slot, or auto-equip):
   `Player::EquipItem` (PlayerStorage.cpp:2824) calls `_ApplyItemMods(pItem, slot, true)` (PlayerStorage.cpp:2846), which calls `ApplyEnchantment(item, apply)` (Player.cpp:6614 inside `_ApplyItemMods` at Player.cpp:6582).

2. **Player logs in with the item already equipped** (e.g. dropped while equipped, or written via GM `.warforged force` while equipped — see footgun below):
   `Player::_LoadInventory` (PlayerStorage.cpp:5925) finishes by calling `_ApplyAllItemMods()` at PlayerStorage.cpp:6054, defined at Player.cpp:7607. That function iterates `m_items[i]` for all equipment slots and calls `ApplyEnchantment(m_items[i], true)` at Player.cpp:7650.

There is no case where mod-warforged needs to invoke `Player::ApplyEnchantment` itself — every realistic write happens at drop time (item not yet equipped) and the equip pipeline picks it up.

### Function signatures

Verbatim from the pinned AC source:

```cpp
// src/server/game/Entities/Item/Item.cpp:920
void Item::SetEnchantment(EnchantmentSlot slot, uint32 id, uint32 duration, uint32 charges, ObjectGuid caster /*= ObjectGuid::Empty*/)
```

```cpp
// src/server/game/Entities/Player/PlayerStorage.cpp:4391
void Player::ApplyEnchantment(Item* item, bool apply)

// src/server/game/Entities/Player/PlayerStorage.cpp:4397
void Player::ApplyEnchantment(Item* item, EnchantmentSlot slot, bool apply, bool apply_dur, bool ignore_condition)
```

```cpp
// src/server/game/Entities/Player/Player.cpp:6582
void Player::_ApplyItemMods(Item* item, uint8 slot, bool apply)

// src/server/game/Entities/Player/Player.cpp:7607
void Player::_ApplyAllItemMods()
```

Declarations live at `src/server/game/Entities/Player/Player.h:1411-1412` (the two `ApplyEnchantment` overloads) and `Player.h:2248` (`_ApplyItemMods`).

### Footgun to avoid

This is the **kb_6aa9f786** footgun the mod-playerbots equip code hit. If a caller bypasses the equip pipeline — e.g. a direct DB UPDATE on `item_instance.enchantments` while the bot/player is logged in and that item is already equipped — `Player::ApplyEnchantment` will never be invoked for the new id. Consequences:

- **Item in a bag at write-time (the normal drop path):** safe. The next `EquipItem` → `_ApplyItemMods` → `ApplyEnchantment` chain picks it up on equip.
- **Item already equipped at write-time, player still logged in:** stat bump does NOT take effect until the player unequips and re-equips, OR logs out + back in (next `_LoadInventory` → `_ApplyAllItemMods` re-applies via Player.cpp:7650).
- **Item already equipped at write-time, player offline:** safe. Next login runs `_ApplyAllItemMods` and the bump activates.

For mod-warforged this means: the `.warforged force` GM command on an already-equipped item is the one case where the player will need to re-equip (or relog) to see the change. Document this in the GM command help string. The `Item::SetEnchantment` call on a brand-new loot-pipeline `Item*` is always safe because the item is by definition not equipped yet — the player has not even seen it.

