#include "Adapters/ObsGetXpAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"

#include <cmath>

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetXp(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "obs.get_xp: target_guid (int) required";
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

        uint32 cur  = p->GetUInt32Value(PLAYER_XP);
        uint32 next = p->GetUInt32Value(PLAYER_NEXT_LEVEL_XP);
        uint8  lvl  = p->GetLevel();
        uint32 rested = static_cast<uint32>(p->GetRestBonus());

        nlohmann::json out = {
            {"level",         lvl},
            {"current_xp",    cur},
            {"next_level_xp", next},
            {"rested_xp",     rested},
        };

        if (next == 0)
        {
            out["pct_to_next"] = nullptr;
        }
        else
        {
            double pct = std::round(static_cast<double>(cur) / static_cast<double>(next) * 1000.0) / 10.0;
            out["pct_to_next"] = pct;
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = out;
        return res;
    }
}
