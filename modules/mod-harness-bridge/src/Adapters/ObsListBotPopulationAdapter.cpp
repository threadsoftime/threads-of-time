// SPDX-License-Identifier: GPL-2.0-or-later
#include "Adapters/ObsListBotPopulationAdapter.h"

#include "Bot/RandomPlayerbotMgr.h"    // sRandomPlayerbotMgr, GetAllBots, PlayerBotMap
#include "Entities/Player/Player.h"    // Player, GetGroup, GetMapId, etc.
#include "Groups/Group.h"              // Group, isRaidGroup, GetGUID
#include "Script/Playerbots.h"         // GET_PLAYERBOT_AI, PlayerbotAI

namespace HarnessBridge::Adapters
{
    DispatchResult ObsListBotPopulation(nlohmann::json const& args)
    {
        DispatchResult res;

        // obs.list_bot_population takes no arguments — the args parameter
        // exists only because all adapters share the same function signature.
        (void)args;

        // GetAllBots() returns PlayerBotMap BY VALUE (a copy of playerBots).
        // Iterating the copy is safe — the live map is not held under lock
        // for the duration of the loop.
        PlayerBotMap bots = sRandomPlayerbotMgr.GetAllBots();

        nlohmann::json bot_array = nlohmann::json::array();

        for (auto const& [guid, botPlayer] : bots)
        {
            if (!botPlayer || !botPlayer->IsInWorld())
                continue;

            nlohmann::json entry = {
                {"bot_guid",      botPlayer->GetGUID().GetCounter()},
                {"name",          botPlayer->GetName()},
                {"map_id",        static_cast<int>(botPlayer->GetMapId())},
                {"x",             botPlayer->GetPositionX()},
                {"y",             botPlayer->GetPositionY()},
                {"z",             botPlayer->GetPositionZ()},
                {"level",         static_cast<int>(botPlayer->GetLevel())},
                {"in_pvp_combat", botPlayer->IsInCombat() && botPlayer->IsPvP()},
            };

            // party_guid: set if the bot is in a non-raid group.
            // raid_guid:  set if the bot is in a raid group.
            Group* grp = botPlayer->GetGroup();
            if (grp)
            {
                uint64_t group_guid = grp->GetGUID().GetCounter();
                if (grp->isRaidGroup())
                    entry["raid_guid"] = group_guid;
                else
                    entry["party_guid"] = group_guid;
            }

            // master_guid: set if the bot has a human master (i.e. is a
            // player-owned bot rather than a free-roaming random bot).
            PlayerbotAI* ai = GET_PLAYERBOT_AI(botPlayer);
            if (ai)
            {
                Player* master = ai->GetMaster();
                if (master)
                    entry["master_guid"] = master->GetGUID().GetCounter();
            }

            bot_array.push_back(std::move(entry));
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"bots", bot_array}};
        return res;
    }
}
