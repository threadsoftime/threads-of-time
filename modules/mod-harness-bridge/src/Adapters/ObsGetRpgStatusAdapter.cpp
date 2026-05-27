#include "Adapters/ObsGetRpgStatusAdapter.h"

#include "Ai/World/Rpg/NewRpgInfo.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "Script/Playerbots.h"

namespace HarnessBridge::Adapters
{
    static char const* RpgStatusName(NewRpgStatus s)
    {
        switch (s)
        {
            case RPG_IDLE:          return "idle";
            case RPG_GO_GRIND:      return "go_grind";
            case RPG_GO_CAMP:       return "go_camp";
            case RPG_WANDER_RANDOM: return "wander_random";
            case RPG_WANDER_NPC:    return "wander_npc";
            case RPG_DO_QUEST:      return "do_quest";
            case RPG_TRAVEL_FLIGHT: return "travel_flight";
            case RPG_REST:          return "rest";
            case RPG_OUTDOOR_PVP:   return "outdoor_pvp";
            case RPG_GO_AH_VISIT:   return "go_ah_visit";
            case RPG_STATUS_END:    return "unknown";  // sentinel — should not appear
            default:                return "unknown";
        }
    }

    DispatchResult ObsGetRpgStatus(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "obs.get_rpg_status: target_guid (int) required";
            return res;
        }
        uint64_t target = args["target_guid"].get<uint64_t>();

        Player* p = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(target));
        if (!p)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "no player with guid " + std::to_string(target) + " online";
            return res;
        }

        PlayerbotAI* ai = GET_PLAYERBOT_AI(p);
        if (!ai)
        {
            res.outcome = DispatchResult::Outcome::Ok;
            res.result_json = {{"is_bot", false}};
            return res;
        }

        NewRpgStatus s = ai->rpgInfo.GetStatus();
        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"is_bot",       true},
            {"status_code",  static_cast<int>(s)},
            {"status_label", RpgStatusName(s)},
            {"description",  ai->rpgInfo.ToString()},
        };
        return res;
    }
}
