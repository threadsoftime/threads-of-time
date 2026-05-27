#ifndef MOD_WARFORGED_RNG_H
#define MOD_WARFORGED_RNG_H

#include <cstdint>

namespace ModWarforged::Rng
{
    // ---------------------------------------------------------------------
    // Pure decision functions. These take the random roll value and the
    // threshold as explicit inputs so they can be unit-tested off-server
    // without pulling in AC's urand() or gConfig.
    //
    // Defined inline in the header so the laptop doctest target can link
    // against them without also needing to compile WarforgedRng.cpp
    // (which #includes AC's Util.h and WarforgedConfig.h).
    //
    // Contract: a roll is a "hit" iff rollValue < threshold.
    //   - threshold == 0  → always returns false (procs disabled).
    //   - threshold == 100 → always returns true so long as production
    //     code passes urand(0, 99) (max value 99 < 100).
    //   - The Warforged + Socket variants are deliberately identical in
    //     logic; we keep two functions to (a) make call sites self-
    //     documenting and (b) leave room for divergent rules in v1.1+.
    // ---------------------------------------------------------------------

    inline bool RollWarforgedWith(std::uint32_t rollValue, std::uint8_t threshold)
    {
        return rollValue < static_cast<std::uint32_t>(threshold);
    }

    inline bool RollSocketWith(std::uint32_t rollValue, std::uint8_t threshold)
    {
        return rollValue < static_cast<std::uint32_t>(threshold);
    }

    // ---------------------------------------------------------------------
    // Production wrappers (defined in WarforgedRng.cpp). These read
    // gConfig + AC's urand() and dispatch to the pure functions above.
    // Not buildable on the laptop (require AC's Util.h + WarforgedConfig
    // linkage). Built on Heimdal as part of the worldserver module bundle.
    // ---------------------------------------------------------------------

    bool RollWarforged();
    bool RollSocket();
}

#endif // MOD_WARFORGED_RNG_H
