#ifndef MOD_BRACKET_SETS_CONFIG_H
#define MOD_BRACKET_SETS_CONFIG_H

#include "Common.h"

namespace BracketSets
{
    struct Config
    {
        bool Enabled = true;
    };

    Config const& GetConfig();
    void LoadConfig();
}

#endif // MOD_BRACKET_SETS_CONFIG_H
