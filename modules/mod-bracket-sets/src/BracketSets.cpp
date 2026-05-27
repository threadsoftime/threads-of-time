#include "BracketSets.h"

#include "BracketSetsBonusEffects.h"
#include "BracketSetsCommandScript.h"
#include "BracketSetsConfig.h"
#include "BracketSetsManager.h"
#include "BracketSetsPersonalLoot.h"
#include "BracketSetsPlayerScript.h"
#include "BracketSetsRegistry.h"
#include "Log.h"
#include "ScriptMgr.h"

namespace
{
    class BracketSetsWorldScript : public WorldScript
    {
    public:
        BracketSetsWorldScript() : WorldScript("BracketSetsWorldScript") { }

        void OnAfterConfigLoad(bool /*reload*/) override
        {
            BracketSets::LoadConfig();
        }

        void OnStartup() override
        {
            if (!BracketSets::GetConfig().Enabled)
            {
                LOG_INFO("server.loading", "[mod-bracket-sets] disabled by config");
                return;
            }

            BracketSets::LoadBonusMap();
            BracketSets::LoadItemsetMap();
            BracketSets::LoadPersonalLootMap();
            LOG_INFO("server.loading",
                     "[mod-bracket-sets] ready ({} bonus mappings loaded for bracket 1)",
                     BracketSets::BonusCount());
        }
    };
}

void AddBracketSetsScripts()
{
    new BracketSetsWorldScript();
    BracketSets::RegisterPlayerScript();
    BracketSets::RegisterCommandScript();
    BracketSets::RegisterBonusEffects();
    BracketSets::RegisterPersonalLootScript();
}
