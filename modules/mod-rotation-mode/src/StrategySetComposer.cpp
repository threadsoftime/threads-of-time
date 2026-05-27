#include "StrategySetComposer.h"

namespace RotationMode
{
    StrategySet Compose(uint8 /*class_id*/, uint8 /*spec_id*/, Mode /*mode*/)
    {
        // v0 scaffold: empty lists. Step 3 fills in the 27 class+spec maps:
        //
        //   rotation mode example (Holy Priest, class 5, spec 2):
        //     on  = {"combat", "healpriest", "holypriest", "conserve mana",
        //            "racials", "use potions", "dead", "nc"}
        //     off = {"melee", "ranged", "follow master", "kite", "flee",
        //            "runaway", "grinding", "travel", "return", "pull",
        //            "stay", "move from group", "dps assist", "tank assist",
        //            "attack enemy players", "aggressive", "focus target",
        //            "wait for attack", "loot non combat"}
        //
        //   grind mode (any class): enables movement + assist + loot
        //   strategies; disables manual-style "rotation only" gating.
        //
        //   squad-mirror (any class): leaves player drive intact; ensures
        //   bot squad's reactions strategy is active.
        //
        //   off: empty lists; mod releases AI control (.playerbots bot self
        //   off).
        return {};
    }
}
