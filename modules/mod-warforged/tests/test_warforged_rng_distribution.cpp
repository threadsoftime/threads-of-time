// SERVER-SIDE BUILD ONLY — DO NOT add to modules/mod-warforged/tests/CMakeLists.txt.
//
// This file exercises the *production* wrappers RollWarforged() and
// RollSocket() against AC's real urand() and the real gConfig. It cannot
// build on the laptop (requires Common.h + Util.cpp + WarforgedConfig.cpp
// linkage) and is intentionally excluded from the standalone doctest
// target.
//
// On the full worldserver build, build it via the AC module test scaffolding
// (Task 10) or manually:
//
//   cd /azerothcore && g++ -std=c++17 \
//     -I src/server/shared -I src/server/game/Util -I src/common \
//     modules/mod-warforged/tests/test_warforged_rng_distribution.cpp \
//     modules/mod-warforged/src/WarforgedRng.cpp \
//     modules/mod-warforged/src/WarforgedConfig.cpp \
//     -L build/lib -lcommon -lshared -lpthread \
//     -o /tmp/test_warforged_rng_distribution
//   /tmp/test_warforged_rng_distribution
//
// Expected output (deterministic seed not used — RNG is real urand, so
// values jitter but tolerances are wide enough to never fail):
//   RollWarforged hit rate at 10%:   ~0.100 (±0.5%)
//   RollSocket    hit rate at 10%:   ~0.100 (±0.5%)
//   P(both)   ≈ 0.010 (±0.2pp)
//   P(neither) ≈ 0.810 (±0.5pp)

#include "../src/WarforgedRng.h"
#include "../src/WarforgedConfig.h"

#include <cassert>
#include <iostream>

using namespace ModWarforged;

static void test_roll_warforged_distribution_at_10_percent()
{
    gConfig.enable = true;
    gConfig.procChance = 10;

    int hits = 0;
    constexpr int N = 100000;
    for (int i = 0; i < N; ++i)
        if (Rng::RollWarforged()) ++hits;

    double rate = double(hits) / N;
    std::cout << "RollWarforged hit rate at 10%: " << rate << "\n";
    assert(rate > 0.095 && rate < 0.105);  // ±0.5% tolerance over 100k samples
}

static void test_roll_socket_distribution_at_10_percent()
{
    gConfig.enable = true;
    gConfig.socketChance = 10;

    int hits = 0;
    constexpr int N = 100000;
    for (int i = 0; i < N; ++i)
        if (Rng::RollSocket()) ++hits;

    double rate = double(hits) / N;
    std::cout << "RollSocket hit rate at 10%: " << rate << "\n";
    assert(rate > 0.095 && rate < 0.105);
}

static void test_rolls_are_independent()
{
    gConfig.enable = true;
    gConfig.procChance = 10;
    gConfig.socketChance = 10;

    int both = 0, neither = 0;
    constexpr int N = 100000;
    for (int i = 0; i < N; ++i)
    {
        bool wf  = Rng::RollWarforged();
        bool soc = Rng::RollSocket();
        if (wf && soc)         ++both;
        else if (!wf && !soc)  ++neither;
    }

    double bothRate    = double(both)    / N;
    double neitherRate = double(neither) / N;
    std::cout << "P(both):    " << bothRate    << " (expect ~0.010)\n";
    std::cout << "P(neither): " << neitherRate << " (expect ~0.810)\n";

    // P(both) = 0.10 * 0.10 = 0.01; P(neither) = 0.90 * 0.90 = 0.81
    assert(bothRate    > 0.008 && bothRate    < 0.012);  // ±0.2pp
    assert(neitherRate > 0.805 && neitherRate < 0.815);  // ±0.5pp
}

int main()
{
    test_roll_warforged_distribution_at_10_percent();
    test_roll_socket_distribution_at_10_percent();
    test_rolls_are_independent();
    std::cout << "All WarforgedRng distribution tests passed.\n";
    return 0;
}
