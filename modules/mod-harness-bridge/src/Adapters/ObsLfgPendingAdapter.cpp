// ObsLfgPendingAdapter — obs.lfg_pending
//
// Drains pending LFG join intents from LfgIntentStore (main-thread-only store).
// Runs on OnTickDrain — safe to read the store without locking.
//
// C++ adapter canonical field set (must match ObsLfgPendingArgs in tool_schemas.py):
//   args.value("max", 64)   — optional u32 cap on how many intents to drain
//
// Response shape consumed by harness.rs::lfg_pending:
//   { "pending": [ { "guid":<u64>, "team_id":<u8>, "roles":<u32>,
//                    "dungeon_ids":[<u32>...], "comment":"<str>", "is_bot":<bool> } ] }

#include "Adapters/ObsLfgPendingAdapter.h"
#include "LfgIntentStore.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsLfgPending(nlohmann::json const& args)
    {
        DispatchResult res;

        // `max` is optional; default 64.  No hard-required fields.
        std::size_t const max = static_cast<std::size_t>(
            args.value("max", 64));

        auto intents = DrainLfgIntents(max);

        // Serialize into the shape the Rust slice expects:
        //   result.pending[] with exact field names as declared in harness.rs.
        nlohmann::json pending = nlohmann::json::array();
        for (auto const& intent : intents)
        {
            nlohmann::json dungeon_arr = nlohmann::json::array();
            for (uint32_t d : intent.dungeon_ids)
                dungeon_arr.push_back(d);

            pending.push_back({
                {"guid",        intent.guid_low},
                {"team_id",     intent.team_id},
                {"roles",       intent.roles},
                {"dungeon_ids", std::move(dungeon_arr)},
                {"comment",     intent.comment},
                {"is_bot",      intent.is_bot},
            });
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"pending", std::move(pending)}};
        return res;
    }

} // namespace HarnessBridge::Adapters
