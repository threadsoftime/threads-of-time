#include "WarforgedRng.h"
#include "WarforgedConfig.h"
#include "Util.h"  // AC's urand()

namespace ModWarforged::Rng
{
    // Production wrappers — the pure decision logic lives inline in the
    // header. These call urand() once per roll, so socket procs are
    // statistically independent of warforged procs.

    bool RollWarforged()
    {
        if (!gConfig.enable) return false;
        return RollWarforgedWith(urand(0, 99), gConfig.procChance);
    }

    bool RollSocket()
    {
        if (!gConfig.enable) return false;
        return RollSocketWith(urand(0, 99), gConfig.socketChance);
    }
}
