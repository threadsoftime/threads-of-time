// test_warforged_picker.cpp — doctest coverage for ModWarforged::EnchantPicker::Pick.
//
// We include the real WarforgedEnchantPicker.h directly. Like WarforgedRng.h
// (Task 2), the picker header is deliberately standalone — only <cstdint>,
// no AC includes. If anyone adds `#include "WarforgedConstants.h"` or any
// AC pull-in to WarforgedEnchantPicker.h, this test target will stop
// building on the laptop. That's a signal to keep the dependency confined
// to WarforgedEnchantPicker.cpp (which is excluded from the laptop target).
//
// NOTE: the DOCTEST_CONFIG_IMPLEMENT_WITH_MAIN macro is already set by
// test_warforged_rng.cpp — this TU only includes <doctest/doctest.h>.

#include <doctest/doctest.h>

#include <cstdint>

#include "WarforgedEnchantPicker.h"

using ModWarforged::EnchantPicker::Pick;

// =====================================================================
// 7-band × 3-quality matrix (spec §4.1)
// =====================================================================
//
// Every (ilvl, quality) tuple below corresponds to one of the 21 rows in
// data/csv/warforged_enchants.csv. We sample ilvl from the middle of each
// band so band-edge cases (next TEST_CASE) can isolate boundary behaviour.

TEST_CASE("Pick: 7-band x 3-quality matrix — sampled mid-band")
{
    struct Case { std::uint32_t ilvl; std::uint8_t quality; std::uint32_t expected; };
    Case cases[] = {
        // Band 0 (1-25)    → 70001/70002/70003
        {  10, 2, 70001 }, {  10, 3, 70002 }, {  10, 4, 70003 },
        // Band 1 (26-50)   → 70011/70012/70013
        {  38, 2, 70011 }, {  38, 3, 70012 }, {  38, 4, 70013 },
        // Band 2 (51-80)   → 70021/70022/70023
        {  65, 2, 70021 }, {  65, 3, 70022 }, {  65, 4, 70023 },
        // Band 3 (81-115)  → 70031/70032/70033
        { 100, 2, 70031 }, { 100, 3, 70032 }, { 100, 4, 70033 },
        // Band 4 (116-150) → 70041/70042/70043
        { 130, 2, 70041 }, { 130, 3, 70042 }, { 130, 4, 70043 },
        // Band 5 (151-200) → 70051/70052/70053
        { 175, 2, 70051 }, { 175, 3, 70052 }, { 175, 4, 70053 },
        // Band 6 (201+)    → 70061/70062/70063
        { 250, 2, 70061 }, { 250, 3, 70062 }, { 250, 4, 70063 },
    };

    for (auto const& c : cases)
    {
        CAPTURE(c.ilvl);
        CAPTURE(c.quality);
        CHECK(Pick(c.ilvl, c.quality) == c.expected);
    }
}

// =====================================================================
// Band boundary correctness
// =====================================================================
//
// The picker uses inclusive upper bounds (ILVL_BAND_BOUNDS_PICKER_INTERNAL).
// ilvl == bound[i] must land in band i, NOT band i+1. The transition test
// ilvl == bound[i]+1 must land in band i+1. These cases cover every band
// transition in the 7-band ladder.

TEST_CASE("Pick: band edges — upper bound stays in current band")
{
    // Upper bound of each band falls in that band (inclusive).
    // Probe at quality 4 (Epic) so the +3 suffix is unambiguous.
    CHECK(Pick( 25, 4) == 70003);  // band 0 upper
    CHECK(Pick( 50, 4) == 70013);  // band 1 upper
    CHECK(Pick( 80, 4) == 70023);  // band 2 upper
    CHECK(Pick(115, 4) == 70033);  // band 3 upper
    CHECK(Pick(150, 4) == 70043);  // band 4 upper
    CHECK(Pick(200, 4) == 70053);  // band 5 upper
}

TEST_CASE("Pick: band edges — upper bound + 1 jumps to next band")
{
    // One step above each upper bound rolls into the next band.
    CHECK(Pick( 26, 4) == 70013);  // band 0 → band 1
    CHECK(Pick( 51, 4) == 70023);  // band 1 → band 2
    CHECK(Pick( 81, 4) == 70033);  // band 2 → band 3
    CHECK(Pick(116, 4) == 70043);  // band 3 → band 4
    CHECK(Pick(151, 4) == 70053);  // band 4 → band 5
    CHECK(Pick(201, 4) == 70063);  // band 5 → band 6
}

TEST_CASE("Pick: band edges — lower bound of each band")
{
    // Lower bound of each band — first ilvl that maps to band i.
    // Band 0's lower bound is 1 (ilvl 0 is the "no item" sentinel).
    CHECK(Pick(  1, 2) == 70001);  // band 0 lower (uncommon)
    CHECK(Pick( 26, 2) == 70011);  // band 1 lower
    CHECK(Pick( 51, 2) == 70021);  // band 2 lower
    CHECK(Pick( 81, 2) == 70031);  // band 3 lower
    CHECK(Pick(116, 2) == 70041);  // band 4 lower
    CHECK(Pick(151, 2) == 70051);  // band 5 lower
    CHECK(Pick(201, 2) == 70061);  // band 6 lower
}

// =====================================================================
// Edge cases — invalid inputs return 0 (no proc)
// =====================================================================

TEST_CASE("Pick: ilvl == 0 returns 0 (no item)")
{
    // Every quality returns 0 when ilvl is 0.
    CHECK(Pick(0, 2) == 0);
    CHECK(Pick(0, 3) == 0);
    CHECK(Pick(0, 4) == 0);
    // Including invalid qualities, for defence in depth.
    CHECK(Pick(0, 0) == 0);
    CHECK(Pick(0, 1) == 0);
    CHECK(Pick(0, 5) == 0);
}

TEST_CASE("Pick: unsupported quality returns 0 (only Uncommon/Rare/Epic proc)")
{
    // Spec §4 — only ITEM_QUALITY_UNCOMMON(2), RARE(3), EPIC(4) proc.
    // POOR(0), COMMON(1), LEGENDARY(5), ARTIFACT(6), HEIRLOOM(7) → 0.
    for (std::uint32_t ilvl : { 10u, 50u, 100u, 200u, 999u })
    {
        CAPTURE(ilvl);
        CHECK(Pick(ilvl, 0) == 0);  // Poor
        CHECK(Pick(ilvl, 1) == 0);  // Common
        CHECK(Pick(ilvl, 5) == 0);  // Legendary
        CHECK(Pick(ilvl, 6) == 0);  // Artifact
        CHECK(Pick(ilvl, 7) == 0);  // Heirloom
    }
}

TEST_CASE("Pick: ilvl 999 quality 4 lands in top band (per task spec)")
{
    // Sanity: very-high ilvl is clamped into band 6, not lost.
    CHECK(Pick(999, 4) == 70063);
    CHECK(Pick(999, 3) == 70062);
    CHECK(Pick(999, 2) == 70061);
}

// =====================================================================
// Algebraic property — return value is always 70000 + band*10 + (q-1)
// =====================================================================

TEST_CASE("Pick: return value decomposes into (band, quality) cleanly")
{
    // For every (band, quality) pair we expect Pick to produce an ID
    // whose suffix encodes the quality (1..3) and whose tens digit
    // encodes the band (0..6). This is the algebraic contract that
    // downstream consumers (e.g. announce code reading the suffix to
    // get quality) rely on.
    std::uint32_t mid_ilvls[7] = { 10, 38, 65, 100, 130, 175, 250 };
    for (std::uint8_t band = 0; band < 7; ++band)
    {
        for (std::uint8_t quality = 2; quality <= 4; ++quality)
        {
            CAPTURE(band);
            CAPTURE(quality);
            std::uint32_t got = Pick(mid_ilvls[band], quality);
            CHECK(got == 70000u + (band * 10u) + (quality - 1u));
            // Decompose:
            CHECK((got - 70000u) / 10u == band);
            CHECK(((got - 70000u) % 10u) + 1u == quality);
        }
    }
}
