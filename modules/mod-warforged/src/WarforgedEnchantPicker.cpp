// WarforgedEnchantPicker.cpp
//
// The Pick() function is defined inline in WarforgedEnchantPicker.h
// (header-only pure logic, no AC dependency — laptop-buildable). This
// translation unit exists solely to enforce the sync contract between
// WarforgedEnchantPicker's internal ILVL_BAND_BOUNDS_PICKER_INTERNAL
// and the canonical WarforgedConstants::ILVL_BAND_BOUNDS at compile
// time on the Heimdal build, where both headers are available.
//
// If the two arrays drift, the static_assert below fails the worldserver
// build immediately — before any in-game effect surfaces. The laptop
// doctest target does NOT compile this .cpp (it would pull in Common.h
// transitively via WarforgedConstants.h); the cross-check fires only on
// Heimdal.

#include "WarforgedEnchantPicker.h"
#include "WarforgedConstants.h"

namespace ModWarforged::EnchantPicker
{
    // Enforce the duplication contract documented in
    // WarforgedEnchantPicker.h. If anyone edits one array without the
    // other, this fails to compile.
    static_assert(ILVL_BAND_BOUNDS_PICKER_INTERNAL[0] == ::ModWarforged::ILVL_BAND_BOUNDS[0],
                  "ILVL_BAND_BOUNDS_PICKER_INTERNAL[0] drifted from WarforgedConstants::ILVL_BAND_BOUNDS[0]");
    static_assert(ILVL_BAND_BOUNDS_PICKER_INTERNAL[1] == ::ModWarforged::ILVL_BAND_BOUNDS[1],
                  "ILVL_BAND_BOUNDS_PICKER_INTERNAL[1] drifted from WarforgedConstants::ILVL_BAND_BOUNDS[1]");
    static_assert(ILVL_BAND_BOUNDS_PICKER_INTERNAL[2] == ::ModWarforged::ILVL_BAND_BOUNDS[2],
                  "ILVL_BAND_BOUNDS_PICKER_INTERNAL[2] drifted from WarforgedConstants::ILVL_BAND_BOUNDS[2]");
    static_assert(ILVL_BAND_BOUNDS_PICKER_INTERNAL[3] == ::ModWarforged::ILVL_BAND_BOUNDS[3],
                  "ILVL_BAND_BOUNDS_PICKER_INTERNAL[3] drifted from WarforgedConstants::ILVL_BAND_BOUNDS[3]");
    static_assert(ILVL_BAND_BOUNDS_PICKER_INTERNAL[4] == ::ModWarforged::ILVL_BAND_BOUNDS[4],
                  "ILVL_BAND_BOUNDS_PICKER_INTERNAL[4] drifted from WarforgedConstants::ILVL_BAND_BOUNDS[4]");
    static_assert(ILVL_BAND_BOUNDS_PICKER_INTERNAL[5] == ::ModWarforged::ILVL_BAND_BOUNDS[5],
                  "ILVL_BAND_BOUNDS_PICKER_INTERNAL[5] drifted from WarforgedConstants::ILVL_BAND_BOUNDS[5]");
    static_assert(ILVL_BAND_BOUNDS_PICKER_INTERNAL[6] == ::ModWarforged::ILVL_BAND_BOUNDS[6],
                  "ILVL_BAND_BOUNDS_PICKER_INTERNAL[6] drifted from WarforgedConstants::ILVL_BAND_BOUNDS[6]");
}
