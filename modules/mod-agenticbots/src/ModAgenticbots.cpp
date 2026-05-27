// SPDX-License-Identifier: GPL-2.0-or-later
#include "ScriptMgr.h"
#include "Log.h"
#include "AgenticbotsConstants.h"

class ModAgenticbotsWorldScript : public WorldScript
{
public:
    ModAgenticbotsWorldScript() : WorldScript("ModAgenticbotsWorldScript") {}

    void OnStartup() override
    {
        LOG_INFO("module", "[mod-agenticbots] loaded, version {}", Agenticbots::VERSION);
    }
};

void Addmod_agenticbotsScripts()
{
    new ModAgenticbotsWorldScript();
}
