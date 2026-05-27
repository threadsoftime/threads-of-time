#include "Adapters/ObsGetMoneyAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetMoney(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "obs.get_money: target_guid (int) required";
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

        uint32 c = p->GetMoney();
        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"copper_total", c},
            {"gold",         c / 10000u},
            {"silver",       (c % 10000u) / 100u},
            {"copper",       c % 100u},
        };
        return res;
    }
}
