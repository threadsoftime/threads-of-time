#ifndef MOD_ROTATION_MODE_STRATEGY_SET_COMPOSER_H
#define MOD_ROTATION_MODE_STRATEGY_SET_COMPOSER_H

#include "Common.h"
#include "HeimbotSettings.h"

#include <string>
#include <vector>

namespace RotationMode
{
    // Maps (class, spec, mode) -> (ON-strategies, OFF-strategies).
    //
    // For Holy Priest in rotation mode, ON includes "combat, healpriest,
    // holypriest, conserve mana, racials, use potions, dead, nc" and OFF
    // includes "melee, ranged, follow master, kite, flee, runaway,
    // grinding, travel, return, pull, stay, move from group, dps assist,
    // tank assist, attack enemy players, aggressive, focus target,
    // wait for attack, loot non combat".
    //
    // v0 scaffold: returns empty lists. Step 3 of PE2 implementation
    // authors the 27 class+spec configurations per mode.

    struct StrategySet
    {
        std::vector<std::string> on;
        std::vector<std::string> off;
    };

    StrategySet Compose(uint8 class_id, uint8 spec_id, Mode mode);
}

#endif // MOD_ROTATION_MODE_STRATEGY_SET_COMPOSER_H
