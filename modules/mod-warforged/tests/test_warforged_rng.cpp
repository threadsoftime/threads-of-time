#define DOCTEST_CONFIG_IMPLEMENT_WITH_MAIN
#include <doctest/doctest.h>

#include <cstdint>

// We include the real WarforgedRng.h here — it's deliberately written to
// be standalone (only <cstdint>, no AC headers) so the pure decision
// functions can be unit-tested off-server. If anyone ever adds an
// `#include "Common.h"` (or similar AC pull-in) to WarforgedRng.h, this
// test target will stop building on the laptop — that's a signal to
// move the new dependency into WarforgedRng.cpp instead.
#include "WarforgedRng.h"

// Spec-reference shadow of ModWarforged::Config. The real Config struct
// lives in src/WarforgedConfig.h, which pulls in Common.h (and the rest
// of the AC type system) and so can't be included on the laptop. The
// shadow exists for two reasons:
//   1. Documents the defaults the production wrappers consume — if
//      WarforgedConfig.h changes (e.g. procChance default bumped from
//      10 to 15), this test will start failing on the spec-defaults
//      assertion below, prompting an intentional spec update.
//   2. Provides a Config value we can feed to RollWarforgedWith /
//      RollSocketWith via the .procChance / .socketChance fields,
//      mirroring how the production wrappers wire it up.
//
// CAVEAT: this shadow does NOT prevent drift between WarforgedConfig.h
// and the shadow itself. Server-side integration validation is what
// surfaces that. (Same caveat as mod-harness-bridge/tests/test_config.cpp.)
namespace ModWarforged
{
    struct Config
    {
        bool          enable             = true;
        std::uint8_t  procChance         = 10;
        std::uint8_t  socketChance       = 10;
        std::uint8_t  minQuality         = 2;
        std::uint8_t  maxQuality         = 4;
        std::uint8_t  announceChannel    = 2;
        bool          playLootSound      = true;
        std::uint32_t lootSoundId        = 3175;
        bool          warnMissingPatch   = false;
    };
}

using ModWarforged::Rng::RollWarforgedWith;
using ModWarforged::Rng::RollSocketWith;

TEST_CASE("Config shadow defaults match spec — Warforged §4")
{
    // If this fails, WarforgedConfig.h has drifted from the spec. Either
    // update the spec + this shadow together, or revert the .h change.
    ModWarforged::Config c;
    CHECK(c.enable == true);
    CHECK(c.procChance == 10);
    CHECK(c.socketChance == 10);
    CHECK(c.minQuality == 2);
    CHECK(c.maxQuality == 4);
}

// =====================================================================
// RollWarforgedWith — pure-function correctness
// =====================================================================

TEST_CASE("RollWarforgedWith: threshold boundary table")
{
    // Contract: hit iff rollValue < threshold.
    struct Case { std::uint32_t roll; std::uint8_t thresh; bool expected; };
    Case cases[] = {
        // Below threshold → hit
        {  0, 10, true  },   // floor
        {  5, 10, true  },   // mid
        {  9, 10, true  },   // just below

        // At threshold → miss (strict <)
        { 10, 10, false },   // boundary
        { 11, 10, false },   // just above
        { 99, 10, false },   // ceiling of urand(0,99)

        // Different thresholds
        {  0,  1, true  },   // single-point window
        {  1,  1, false },   // single-point boundary
        { 49, 50, true  },   // half
        { 50, 50, false },   // half boundary
    };

    for (auto const& c : cases)
    {
        CAPTURE(c.roll);
        CAPTURE(c.thresh);
        CHECK(RollWarforgedWith(c.roll, c.thresh) == c.expected);
    }
}

TEST_CASE("RollWarforgedWith: threshold 0 disables procs entirely")
{
    // Spec §4 — setting procChance=0 in the config disables procs.
    // The pure function should reflect that for every possible roll.
    for (std::uint32_t roll = 0; roll < 100; ++roll)
    {
        CAPTURE(roll);
        CHECK(RollWarforgedWith(roll, 0) == false);
    }
}

TEST_CASE("RollWarforgedWith: threshold 100 fires on every urand(0,99) draw")
{
    // urand(0, 99) max output is 99; 99 < 100 → always true.
    // (We don't test rollValue >= 100 because production never passes
    // such a value; the pure function's behaviour there is "miss" by
    // contract but it's not a documented use case.)
    for (std::uint32_t roll = 0; roll < 100; ++roll)
    {
        CAPTURE(roll);
        CHECK(RollWarforgedWith(roll, 100) == true);
    }
}

// =====================================================================
// RollSocketWith — pure-function correctness (mirrors warforged)
// =====================================================================

TEST_CASE("RollSocketWith: threshold boundary table")
{
    // Currently identical logic to RollWarforgedWith. The test exists
    // so that if v1.1 introduces divergent rules (e.g. socket procs use
    // a different distribution), this case catches the regression.
    struct Case { std::uint32_t roll; std::uint8_t thresh; bool expected; };
    Case cases[] = {
        {  0, 10, true  },
        {  9, 10, true  },
        { 10, 10, false },
        { 99, 10, false },
        { 49, 50, true  },
        { 50, 50, false },
    };

    for (auto const& c : cases)
    {
        CAPTURE(c.roll);
        CAPTURE(c.thresh);
        CHECK(RollSocketWith(c.roll, c.thresh) == c.expected);
    }
}

TEST_CASE("RollSocketWith: threshold 0 disables socket procs")
{
    for (std::uint32_t roll = 0; roll < 100; ++roll)
    {
        CAPTURE(roll);
        CHECK(RollSocketWith(roll, 0) == false);
    }
}

// =====================================================================
// Independence — at the pure-function level
// =====================================================================

TEST_CASE("RollWarforgedWith and RollSocketWith are independent given independent rolls")
{
    // The pure functions are stateless; given two arbitrary roll values
    // their results depend only on (roll, threshold). This verifies the
    // mathematical contract: production code achieves independence by
    // calling urand() once per function, so as long as urand draws are
    // independent (AC guarantees), the two procs are statistically
    // independent.
    //
    // The server-side distribution test (Task 10+) verifies the empirical
    // joint distribution to ±0.2pp tolerance over 100k samples.

    ModWarforged::Config c;
    c.procChance = 10;
    c.socketChance = 10;

    // Independent rolls — both hit
    CHECK(RollWarforgedWith(3, c.procChance)   == true);
    CHECK(RollSocketWith   (7, c.socketChance) == true);

    // Independent rolls — only warforged hits
    CHECK(RollWarforgedWith(3,  c.procChance)   == true);
    CHECK(RollSocketWith   (50, c.socketChance) == false);

    // Independent rolls — only socket hits
    CHECK(RollWarforgedWith(50, c.procChance)   == false);
    CHECK(RollSocketWith   (3,  c.socketChance) == true);

    // Independent rolls — both miss
    CHECK(RollWarforgedWith(50, c.procChance)   == false);
    CHECK(RollSocketWith   (50, c.socketChance) == false);
}

// =====================================================================
// Wired-via-Config — pure functions consume Config fields correctly
// =====================================================================

TEST_CASE("RollWarforgedWith uses Config.procChance as threshold")
{
    // Mirrors how RollWarforged() in WarforgedRng.cpp passes the value.
    ModWarforged::Config c;
    c.procChance = 25;

    CHECK(RollWarforgedWith(24, c.procChance) == true);
    CHECK(RollWarforgedWith(25, c.procChance) == false);
}

TEST_CASE("RollSocketWith uses Config.socketChance as threshold")
{
    ModWarforged::Config c;
    c.socketChance = 5;

    CHECK(RollSocketWith( 4, c.socketChance) == true);
    CHECK(RollSocketWith( 5, c.socketChance) == false);
    CHECK(RollSocketWith(99, c.socketChance) == false);
}
