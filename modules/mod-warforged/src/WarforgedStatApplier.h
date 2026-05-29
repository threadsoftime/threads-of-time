#ifndef MOD_WARFORGED_STAT_APPLIER_H
#define MOD_WARFORGED_STAT_APPLIER_H

#include <cstdint>

// Forward-declare AC's EnchantmentSlot enum (definition lives in
// src/server/game/Entities/Item/Item.h in the AC tree). Matching the
// underlying type and unscoped form means our constants below are
// the *same type* as AC's enum on server-side builds — no narrowing
// conversion at call sites.
//
// On the laptop doctest build (no AC headers), this forward decl
// stands on its own — the test's MockItem accepts an implicit
// EnchantmentSlot → uint8_t conversion (well-defined for unscoped
// enums backed by uint8_t).
enum EnchantmentSlot : std::uint8_t;

// ---------------------------------------------------------------------
// WarforgedStatApplier — writes Warforged + socket enchants to an Item
// (or any type exposing the same minimal interface).
//
// Design choices (Task 8):
//
// 1. **Header-only templates.** Both entry points are template functions
//    so the same code path runs for the test-side MockItem and AC's real
//    `Item`. Both must expose:
//        uint32 GetEnchantmentId(uint8 slot) const
//        void   SetEnchantment(uint8 slot, uint32 id, uint32 duration, uint32 charges)
//    That is the entire surface — the applier doesn't reach into the
//    item's template or other accessors.
//
// 2. **No AC includes.** Like WarforgedRng.h and WarforgedEnchantPicker.h,
//    this header is intentionally standalone. The slot constants and the
//    Warforged-ID range below are duplicated from WarforgedConstants.h,
//    and a static_assert in WarforgedStatApplier.cpp (built only on the
//    full worldserver build, where Common.h / ItemTemplate.h / Item.h exist) verifies
//    they stay in sync. Same drift-guard pattern as Task 3's picker.
//
//    If you change BONUS_ENCHANTMENT_SLOT, PRISMATIC_ENCHANTMENT_SLOT,
//    PRISMATIC_SOCKET_ENCHANT_ID, or the WF enchant ID range in
//    WarforgedConstants.h, also change the matching constexpr below.
//    WarforgedStatApplier.cpp's static_assert will fail at compile time
//    if they drift.
//
// 3. **No logging in the template.** AC's LOG_WARN macro requires
//    `Log.h`, which transitively pulls in the rest of the AC type system
//    and would break the laptop doctest target. Instead, both functions
//    return a `bool` (true == applied, false == skipped); the caller
//    (WarforgedManager — Task 9) is responsible for logging on `false`
//    if it cares about the diagnostic. This keeps test and production
//    code paths byte-identical.
//
// 4. **ApplySocket takes `inventoryType` as a parameter** rather than
//    reaching into `item.GetTemplate()->InventoryType`. Same reasoning
//    as (1): keep the applier's contract minimal so the mock is trivial.
//    Caller fetches invtype from the real Item before calling.
// ---------------------------------------------------------------------

namespace ModWarforged::StatApplier
{
    // Slot constants — MUST stay in sync with WarforgedConstants.h.
    // WarforgedStatApplier.cpp's static_assert enforces this.
    // Typed as ::EnchantmentSlot (forward-declared above) so AC's
    // Item::SetEnchantment(EnchantmentSlot, ...) overload resolves
    // without narrowing.
    inline constexpr ::EnchantmentSlot WF_BONUS_SLOT  = static_cast<::EnchantmentSlot>(5); // BONUS_ENCHANTMENT_SLOT
    inline constexpr ::EnchantmentSlot WF_SOCKET_SLOT = static_cast<::EnchantmentSlot>(6); // PRISMATIC_ENCHANTMENT_SLOT
    inline constexpr std::uint32_t PRISMATIC_SOCKET_ENCHANT_ID = 3729; // Eternal Belt Buckle uses this

    // Warforged enchant ID range — MUST stay in sync with WarforgedConstants.h.
    inline constexpr std::uint32_t WF_ENCHANT_ID_MIN = 70001;
    inline constexpr std::uint32_t WF_ENCHANT_ID_MAX = 70063;

    // AC's INVTYPE_WAIST value (ItemTemplate.h). Belts use the PRISMATIC slot
    // for Eternal Belt Buckle; we MUST NOT stomp that. MUST stay in sync.
    inline constexpr std::uint8_t INVTYPE_WAIST_VALUE = 6;

    // Returns true if the given enchant ID is one we wrote (a Warforged tier
    // enchant). Used by ApplyWarforged to distinguish "slot occupied by us
    // (re-roll, OK to overwrite)" from "slot occupied by someone else (skip)".
    inline constexpr bool IsWarforgedEnchant(std::uint32_t enchantId)
    {
        return enchantId >= WF_ENCHANT_ID_MIN && enchantId <= WF_ENCHANT_ID_MAX;
    }

    // ApplyWarforged — write `enchantId` to BONUS_ENCHANTMENT_SLOT.
    //
    // Contract:
    //   - If the slot is empty (== 0) → write, return true.
    //   - If the slot holds an existing Warforged enchant → overwrite
    //     (re-roll case), return true.
    //   - If the slot holds a non-Warforged enchant → SKIP, return false.
    //     Caller may log a warning; we don't (see header comment §3).
    //
    // Pure: no globals, no urand, no gConfig. Safe to call from any thread
    // that has exclusive access to `item`.
    template <typename ItemT>
    bool ApplyWarforged(ItemT& item, std::uint32_t enchantId)
    {
        std::uint32_t existing = item.GetEnchantmentId(WF_BONUS_SLOT);
        if (existing != 0 && !IsWarforgedEnchant(existing))
            return false;

        item.SetEnchantment(WF_BONUS_SLOT, enchantId, 0u, 0u);
        return true;
    }

    // ApplySocket — write the prismatic-socket enchant to PRISMATIC_ENCHANTMENT_SLOT.
    //
    // Contract:
    //   - If `inventoryType` == INVTYPE_WAIST (6) → SKIP, return false.
    //     (Eternal Belt Buckle owns this slot on belts.)
    //   - If the slot already holds an enchant → SKIP, return false.
    //   - Otherwise → write PRISMATIC_SOCKET_ENCHANT_ID, return true.
    //
    // Note: we do NOT check whether the existing slot value is one of ours.
    // The prismatic socket enchant is a vanilla AC ID (3729), not a WF ID,
    // and a "re-roll the socket" doesn't change the value anyway. So any
    // non-zero existing value blocks the write.
    template <typename ItemT>
    bool ApplySocket(ItemT& item, std::uint8_t inventoryType)
    {
        if (inventoryType == INVTYPE_WAIST_VALUE)
            return false;

        std::uint32_t existing = item.GetEnchantmentId(WF_SOCKET_SLOT);
        if (existing != 0)
            return false;

        item.SetEnchantment(WF_SOCKET_SLOT, PRISMATIC_SOCKET_ENCHANT_ID, 0u, 0u);
        return true;
    }
}

#endif // MOD_WARFORGED_STAT_APPLIER_H
