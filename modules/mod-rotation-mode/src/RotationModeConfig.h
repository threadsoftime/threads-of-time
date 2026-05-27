#ifndef MOD_ROTATION_MODE_CONFIG_H
#define MOD_ROTATION_MODE_CONFIG_H

#include "Common.h"

namespace RotationMode
{
    struct Config
    {
        bool Enabled = true;
    };

    Config const& GetConfig();
    void LoadConfig();
}

#endif // MOD_ROTATION_MODE_CONFIG_H
