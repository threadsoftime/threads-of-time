// test_warforged_applier.cpp — doctest coverage for ModWarforged::StatApplier.
//
// We include WarforgedStatApplier.h directly. Like the other Task 2/3 headers,
// the applier is intentionally standalone — only <cstdint>, no AC includes.
// The applier's slot constants (WF_BONUS_SLOT, WF_SOCKET_SLOT, PRISMATIC_SOCKET_ENCHANT_ID)
// are duplicated from WarforgedConstants.h with a static_assert sync guard in the
// .cpp (Heimdal-only). If anyone adds `#include "WarforgedConstants.h"` or any
// AC pull-in to WarforgedStatApplier.h, this test target will stop building on
// the laptop. That's a signal to keep the dependency confined to the .cpp.
//
// MockItem mirrors the minimal surface real AC `Item` exposes: GetEnchantmentId
// and SetEnchantment with matching parameter shape. The applier is templated
// so the same code path runs against both MockItem (here) and the real Item
// (production explicit instantiation in the .cpp).
//
// NOTE: the DOCTEST_CONFIG_IMPLEMENT_WITH_MAIN macro is already set by
// test_warforged_rng.cpp — this TU only includes <doctest/doctest.h>.

#include <doctest/doctest.h>

#include <cstdint>
#include <map>

#include "WarforgedStatApplier.h"

using ModWarforged::StatApplier::ApplyWarforged;
using ModWarforged::StatApplier::ApplySocket;
using ModWarforged::StatApplier::WF_BONUS_SLOT;
using ModWarforged::StatApplier::WF_SOCKET_SLOT;
using ModWarforged::StatApplier::PRISMATIC_SOCKET_ENCHANT_ID;

// =====================================================================
// MockItem — minimal surface the applier templates require.
// =====================================================================
//
// Real AC `Item` exposes:
//   uint32 Item::GetEnchantmentId(EnchantmentSlot slot) const
//   void   Item::SetEnchantment(EnchantmentSlot slot, uint32 id, uint32 duration, uint32 charges)
// Both signatures match below (uint8 stands in for the EnchantmentSlot enum
// since the slot is just an integer index 0..MAX_ENCHANTMENT_SLOT).

struct MockItem
{
    std::uint32_t entry = 0;
    std::map<std::uint8_t, std::uint32_t> enchantSlots;  // slot → enchant ID

    std::uint32_t GetEnchantmentId(std::uint8_t slot) const
    {
        auto it = enchantSlots.find(slot);
        return it == enchantSlots.end() ? 0u : it->second;
    }

    void SetEnchantment(std::uint8_t slot, std::uint32_t id,
                        std::uint32_t /*duration*/, std::uint32_t /*charges*/)
    {
        enchantSlots[slot] = id;
    }
};

// AC's INVTYPE_WAIST == 6 (ItemTemplate.h). The applier takes inventoryType as
// a uint8 parameter, so the test file hardcodes the values rather than pulling
// in AC headers (which aren't available on the laptop build).
static constexpr std::uint8_t INVTYPE_HEAD_LOCAL  = 1;
static constexpr std::uint8_t INVTYPE_WAIST_LOCAL = 6;
static constexpr std::uint8_t INVTYPE_CHEST_LOCAL = 5;

// =====================================================================
// ApplyWarforged — BONUS slot write + collision guards
// =====================================================================

TEST_CASE("ApplyWarforged: writes enchant to BONUS slot when slot is empty")
{
    MockItem item;
    item.entry = 12345;

    bool ok = ApplyWarforged(item, 70013u);

    CHECK(ok == true);
    CHECK(item.GetEnchantmentId(WF_BONUS_SLOT) == 70013u);
}

TEST_CASE("ApplyWarforged: skips when BONUS slot holds non-Warforged enchant")
{
    // Some other mod (or a vanilla effect) put an enchant in slot 5 that's
    // NOT in our 70001..70063 range. We must NOT overwrite it.
    MockItem item;
    item.entry = 12345;
    item.enchantSlots[WF_BONUS_SLOT] = 12345u;  // out of WF range

    bool ok = ApplyWarforged(item, 70013u);

    CHECK(ok == false);
    CHECK(item.GetEnchantmentId(WF_BONUS_SLOT) == 12345u);  // unchanged
}

TEST_CASE("ApplyWarforged: overwrites existing Warforged enchant (re-roll case)")
{
    // Re-roll scenario: the slot already holds a Warforged enchant (e.g.,
    // the player previously procced) and we're replacing it with a fresh
    // roll. IsWarforgedEnchant() recognises the ID is ours, so we overwrite.
    MockItem item;
    item.enchantSlots[WF_BONUS_SLOT] = 70013u;  // existing WF band 1 epic

    bool ok = ApplyWarforged(item, 70023u);     // new WF band 2 epic

    CHECK(ok == true);
    CHECK(item.GetEnchantmentId(WF_BONUS_SLOT) == 70023u);
}

TEST_CASE("ApplyWarforged: collision check uses full Warforged ID range")
{
    // Every ID in 70001..70063 is recognised as ours and overwritten.
    // Bracketed sample (min, mid, max) keeps the test cheap.
    for (std::uint32_t existing : { 70001u, 70032u, 70063u })
    {
        CAPTURE(existing);
        MockItem item;
        item.enchantSlots[WF_BONUS_SLOT] = existing;

        bool ok = ApplyWarforged(item, 70043u);

        CHECK(ok == true);
        CHECK(item.GetEnchantmentId(WF_BONUS_SLOT) == 70043u);
    }
}

TEST_CASE("ApplyWarforged: just-outside-range IDs are treated as non-Warforged")
{
    // 70000 (one below WF_ENCHANT_ID_MIN) and 70064 (one above WF_ENCHANT_ID_MAX)
    // are NOT ours and must NOT be overwritten.
    for (std::uint32_t existing : { 70000u, 70064u })
    {
        CAPTURE(existing);
        MockItem item;
        item.enchantSlots[WF_BONUS_SLOT] = existing;

        bool ok = ApplyWarforged(item, 70013u);

        CHECK(ok == false);
        CHECK(item.GetEnchantmentId(WF_BONUS_SLOT) == existing);
    }
}

// =====================================================================
// ApplySocket — PRISMATIC slot write + belt skip + collision guards
// =====================================================================

TEST_CASE("ApplySocket: writes prismatic enchant on non-belt items")
{
    MockItem item;

    bool ok = ApplySocket(item, INVTYPE_HEAD_LOCAL);

    CHECK(ok == true);
    CHECK(item.GetEnchantmentId(WF_SOCKET_SLOT) == PRISMATIC_SOCKET_ENCHANT_ID);
}

TEST_CASE("ApplySocket: skips belts (INVTYPE_WAIST collides with Eternal Belt Buckle)")
{
    // Eternal Belt Buckle uses PRISMATIC_ENCHANTMENT_SLOT on belts to add a
    // socket. We must not stomp it. The applier rejects INVTYPE_WAIST (6).
    MockItem item;

    bool ok = ApplySocket(item, INVTYPE_WAIST_LOCAL);

    CHECK(ok == false);
    CHECK(item.GetEnchantmentId(WF_SOCKET_SLOT) == 0u);  // never written
}

TEST_CASE("ApplySocket: skips when PRISMATIC slot already holds an enchant")
{
    // Someone else (a future mod, or a re-roll) already wrote to the slot —
    // we don't overwrite, regardless of what's in there.
    MockItem item;
    item.enchantSlots[WF_SOCKET_SLOT] = PRISMATIC_SOCKET_ENCHANT_ID;  // already-socketed

    bool ok = ApplySocket(item, INVTYPE_CHEST_LOCAL);

    CHECK(ok == false);
    CHECK(item.GetEnchantmentId(WF_SOCKET_SLOT) == PRISMATIC_SOCKET_ENCHANT_ID);  // unchanged
}

TEST_CASE("ApplySocket: skips on collision regardless of inventoryType")
{
    // Even with a non-belt invtype, a populated slot blocks the write.
    MockItem item;
    item.enchantSlots[WF_SOCKET_SLOT] = 99999u;  // some other enchant ID

    bool ok = ApplySocket(item, INVTYPE_HEAD_LOCAL);

    CHECK(ok == false);
    CHECK(item.GetEnchantmentId(WF_SOCKET_SLOT) == 99999u);
}

TEST_CASE("ApplySocket: writes succeed across various non-belt inventory types")
{
    // Sanity sweep — every non-WAIST invtype should produce a write.
    // INVTYPE_NON_EQUIP(0) is non-equippable but the applier doesn't care
    // about equippability — caller's eligibility filter does. So we expect
    // it to write here too.
    for (std::uint8_t invtype : { 0u, 1u, 2u, 3u, 4u, 5u, 7u, 8u, 9u, 10u })
    {
        CAPTURE(static_cast<int>(invtype));
        MockItem item;

        bool ok = ApplySocket(item, invtype);

        CHECK(ok == true);
        CHECK(item.GetEnchantmentId(WF_SOCKET_SLOT) == PRISMATIC_SOCKET_ENCHANT_ID);
    }
}
