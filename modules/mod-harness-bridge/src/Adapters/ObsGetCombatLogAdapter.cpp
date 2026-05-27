#include "Adapters/ObsGetCombatLogAdapter.h"
#include "CombatLogBuffer.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetCombatLog(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "obs.get_combat_log: target_guid (int) required";
            return res;
        }
        uint64_t target = args["target_guid"].get<uint64_t>();
        uint64_t since  = args.value("since_ts_ms", uint64_t{0});
        std::size_t limit = args.value("limit", std::size_t{200});
        if (limit > 500) limit = 500;

        auto events = Buffer().Slice(target, since, limit);
        nlohmann::json arr = nlohmann::json::array();
        for (auto const& ev : events)
        {
            arr.push_back({
                {"ts_ms",       ev.ts_ms},
                {"spell_id",    ev.spell_id},
                {"source_guid", ev.source_guid},
                {"target_guid", ev.target_guid},
                {"amount",      ev.amount},
                {"kind",        ev.kind},
                {"is_crit",     ev.is_crit},
            });
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"events", arr}};
        return res;
    }
}
