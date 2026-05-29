#ifndef MOD_WARFORGED_ENCHANT_PICKER_H
#define MOD_WARFORGED_ENCHANT_PICKER_H

#include <cstdint>

// ---------------------------------------------------------------------
// WarforgedEnchantPicker — pure (ilvl, quality) → DBC enchant ID lookup.
//
// Design choice (option a from Task 3 pre-flight): this header is
// intentionally standalone. It only depends on <cstdint> and does NOT
// include WarforgedConstants.h. The ilvl band bounds are duplicated
// here as ILVL_BAND_BOUNDS_PICKER_INTERNAL with a sync contract enforced
// by a static_assert in WarforgedEnchantPicker.cpp (server-side build,
// where WarforgedConstants.h is available). This mirrors the Task 2
// WarforgedRng.h pattern: pure logic stays free of AC includes so the
// laptop doctest target can link against the header without compiling
// the .cpp (which transitively pulls Common.h via WarforgedConstants.h).
//
// If you change ILVL_BAND_BOUNDS in WarforgedConstants.h, also change
// ILVL_BAND_BOUNDS_PICKER_INTERNAL below. WarforgedEnchantPicker.cpp's
// static_assert will fail at compile time if they drift.
// ---------------------------------------------------------------------

namespace ModWarforged::EnchantPicker
{
    // Inclusive upper bound of each ilvl band. Index == band number (0..6).
    // Band 0: ilvl 1..25   → enchant IDs 70001/70002/70003
    // Band 1: ilvl 26..50  → enchant IDs 70011/70012/70013
    // Band 2: ilvl 51..80  → enchant IDs 70021/70022/70023
    // Band 3: ilvl 81..115 → enchant IDs 70031/70032/70033
    // Band 4: ilvl 116..150→ enchant IDs 70041/70042/70043
    // Band 5: ilvl 151..200→ enchant IDs 70051/70052/70053
    // Band 6: ilvl 201..   → enchant IDs 70061/70062/70063
    //
    // MUST stay in sync with WarforgedConstants::ILVL_BAND_BOUNDS — see
    // header-level comment above.
    inline constexpr std::uint32_t ILVL_BAND_BOUNDS_PICKER_INTERNAL[7] =
        { 25, 50, 80, 115, 150, 200, 9999 };

    // Returns the DBC enchant ID for the (ilvl, quality) combo, or 0 if
    // the combo is unsupported.
    //
    // Contract (spec §4.1 table):
    //   - ilvl == 0           → 0 (no item)
    //   - quality not in [2,4]→ 0 (only Uncommon/Rare/Epic proc)
    //   - otherwise           → 70000 + band*10 + (quality - 1)
    //     where band is the smallest index i such that
    //     ilvl <= ILVL_BAND_BOUNDS_PICKER_INTERNAL[i].
    //
    // Pure: no globals, no urand, no gConfig. Safe to call from any
    // thread. Defined inline so the laptop doctest target can link
    // against it without compiling WarforgedEnchantPicker.cpp.
    inline std::uint32_t Pick(std::uint32_t ilvl, std::uint8_t quality)
    {
        if (ilvl == 0)               return 0;
        if (quality < 2 || quality > 4) return 0;

        // Find the band index — smallest i s.t. ilvl <= bound[i].
        // Loop is bounded; band 6 (9999) is the catch-all upper.
        std::uint8_t band = 0;
        for (std::uint8_t i = 0; i < 7; ++i)
        {
            if (ilvl <= ILVL_BAND_BOUNDS_PICKER_INTERNAL[i])
            {
                band = i;
                break;
            }
        }

        return 70000u + (static_cast<std::uint32_t>(band) * 10u)
                      + (static_cast<std::uint32_t>(quality) - 1u);
    }
}

#endif // MOD_WARFORGED_ENCHANT_PICKER_H
