#include "RotationModeConfig.h"

#include "Config.h"
#include "Log.h"

namespace RotationMode
{
    namespace
    {
        Config gConfig;
    }

    Config const& GetConfig()
    {
        return gConfig;
    }

    void LoadConfig()
    {
        gConfig.Enabled = sConfigMgr->GetOption<bool>("RotationMode.Enable", true);
        LOG_INFO("server.loading", "[mod-rotation-mode] Enable={}", gConfig.Enabled ? 1 : 0);
    }
}
