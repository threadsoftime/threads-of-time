#include "BracketSetsConfig.h"

#include "Config.h"
#include "Log.h"

namespace BracketSets
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
        gConfig.Enabled = sConfigMgr->GetOption<bool>("BracketSets.Enable", true);
        LOG_INFO("server.loading", "[mod-bracket-sets] Enable={}", gConfig.Enabled ? 1 : 0);
    }
}
