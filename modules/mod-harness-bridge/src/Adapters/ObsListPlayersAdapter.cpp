// SPDX-License-Identifier: GPL-2.0-or-later
#include "Adapters/ObsListPlayersAdapter.h"

#include "Server/WorldSessionMgr.h"   // sWorldSessionMgr, SessionMap
#include "Server/WorldSession.h"       // WorldSession::GetPlayer()
#include "Entities/Player/Player.h"    // Player, GetGroup, IsPvP, IsInCombat
#include "Groups/Group.h"              // Group, isRaidGroup, GetGUID
#include "Script/Playerbots.h"         // GET_PLAYERBOT_AI (to exclude bots)

namespace HarnessBridge::Adapters
{
    DispatchResult ObsListPlayers(nlohmann::json const& args)
    {
        DispatchResult res;

        // obs.list_players takes no arguments — the args parameter exists
        // only because all adapters share the same function signature.
        (void)args;

        WorldSessionMgr::SessionMap const& sessions = sWorldSessionMgr->GetAllSessions();

        nlohmann::json players = nlohmann::json::array();

        for (auto const& [account_id, session] : sessions)
        {
            if (!session)
                continue;

            Player* player = session->GetPlayer();
            if (!player || !player->IsInWorld())
                continue;

            // Exclude bots — only emit real human players.
            if (GET_PLAYERBOT_AI(player) != nullptr)
                continue;

            nlohmann::json entry = {
                {"player_guid", player->GetGUID().GetCounter()},
                {"name",        player->GetName()},
                {"map_id",      static_cast<int>(player->GetMapId())},
                {"x",           player->GetPositionX()},
                {"y",           player->GetPositionY()},
                {"z",           player->GetPositionZ()},
                {"level",       static_cast<int>(player->GetLevel())},
                {"in_pvp_combat", player->IsInCombat() && player->IsPvP()},
            };

            // party_guid: set if the player is in a non-raid group.
            // raid_guid:  set if the player is in a raid group.
            Group* grp = player->GetGroup();
            if (grp)
            {
                uint64_t group_guid = grp->GetGUID().GetCounter();
                if (grp->isRaidGroup())
                    entry["raid_guid"] = group_guid;
                else
                    entry["party_guid"] = group_guid;
            }

            players.push_back(std::move(entry));
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"players", players}};
        return res;
    }
}
