// WarforgedStatApplier.cpp
//
// The ApplyWarforged() and ApplySocket() functions are defined as inline
// templates in WarforgedStatApplier.h (header-only pure logic, no AC
// dependency — laptop-buildable). This translation unit exists solely to
// enforce the sync contract between the applier's internal slot constants
// (WF_BONUS_SLOT, WF_SOCKET_SLOT, PRISMATIC_SOCKET_ENCHANT_ID, WF enchant
// ID range, INVTYPE_WAIST_VALUE) and the canonical declarations in
// WarforgedConstants.h / AC's ItemTemplate.h at compile time on Heimdal,
// where all those headers are available.
//
// If any constant drifts, the static_assert below fails the worldserver
// build immediately — before any in-game effect surfaces. The laptop
// doctest target does NOT compile this .cpp (it would pull in Common.h
// transitively via WarforgedConstants.h); the cross-check fires only on
// Heimdal.
//
// Same drift-guard pattern as WarforgedEnchantPicker.cpp.

#include "WarforgedStatApplier.h"
#include "WarforgedConstants.h"
#include "ItemTemplate.h"   // for INVTYPE_WAIST

namespace ModWarforged::StatApplier
{
    // Slot constants — enforce duplication contract documented in
    // WarforgedStatApplier.h.
    static_assert(WF_BONUS_SLOT == ::ModWarforged::WF_BONUS_SLOT,
                  "StatApplier::WF_BONUS_SLOT drifted from WarforgedConstants::WF_BONUS_SLOT");
    static_assert(WF_SOCKET_SLOT == ::ModWarforged::WF_SOCKET_SLOT,
                  "StatApplier::WF_SOCKET_SLOT drifted from WarforgedConstants::WF_SOCKET_SLOT");
    static_assert(PRISMATIC_SOCKET_ENCHANT_ID == ::ModWarforged::PRISMATIC_SOCKET_ENCHANT_ID,
                  "StatApplier::PRISMATIC_SOCKET_ENCHANT_ID drifted from "
                  "WarforgedConstants::PRISMATIC_SOCKET_ENCHANT_ID");

    // Warforged enchant ID range — enforce duplication contract.
    static_assert(WF_ENCHANT_ID_MIN == ::ModWarforged::WF_ENCHANT_ID_MIN,
                  "StatApplier::WF_ENCHANT_ID_MIN drifted from "
                  "WarforgedConstants::WF_ENCHANT_ID_MIN");
    static_assert(WF_ENCHANT_ID_MAX == ::ModWarforged::WF_ENCHANT_ID_MAX,
                  "StatApplier::WF_ENCHANT_ID_MAX drifted from "
                  "WarforgedConstants::WF_ENCHANT_ID_MAX");

    // INVTYPE_WAIST is owned by AC's ItemTemplate.h. If Blizzard ever
    // renumbers it (won't happen — it's wire-format), this catches it.
    static_assert(INVTYPE_WAIST_VALUE == static_cast<std::uint8_t>(INVTYPE_WAIST),
                  "StatApplier::INVTYPE_WAIST_VALUE drifted from AC's INVTYPE_WAIST");
}
