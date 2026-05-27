#include "RotationMode.h"

#include "HeimbotCommandScript.h"
#include "Log.h"
#include "RotationModeConfig.h"
#include "RotationModePlayerScript.h"
#include "ScriptMgr.h"

namespace
{
    class RotationModeWorldScript : public WorldScript
    {
    public:
        RotationModeWorldScript() : WorldScript("RotationModeWorldScript") { }

        void OnAfterConfigLoad(bool /*reload*/) override
        {
            RotationMode::LoadConfig();
        }

        void OnStartup() override
        {
            if (!RotationMode::GetConfig().Enabled)
            {
                LOG_INFO("server.loading", "[mod-rotation-mode] disabled by config");
                return;
            }
            LOG_INFO("server.loading", "[mod-rotation-mode] ready (scaffold v0)");
        }
    };
}

void AddRotationModeScripts()
{
    new RotationModeWorldScript();
    RotationMode::RegisterPlayerScript();
    RotationMode::RegisterCommandScript();
}
