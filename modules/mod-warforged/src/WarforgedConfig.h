#ifndef MOD_WARFORGED_CONFIG_H
#define MOD_WARFORGED_CONFIG_H

#include "Common.h"

namespace ModWarforged
{
    struct Config
    {
        bool     enable             = true;
        uint8    procChance         = 10;
        uint8    socketChance       = 10;
        uint8    minQuality         = 2;
        uint8    maxQuality         = 4;
        uint8    announceChannel    = 2;
        bool     playLootSound      = true;
        uint32   lootSoundId        = 3175;
        bool     warnMissingPatch   = false;
        uint8    socketMinCharLevel = 56;  // Below this level, socket procs are skipped (no gem economy pre-Outland on this fork)
    };

    // Loaded once at worldserver startup. Read-only after.
    extern Config gConfig;

    void LoadConfig();
}

#endif // MOD_WARFORGED_CONFIG_H
