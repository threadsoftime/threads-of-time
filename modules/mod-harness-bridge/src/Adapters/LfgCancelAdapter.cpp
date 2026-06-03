// LfgCancelAdapter — lfg.cancel
//
// Drains pending LFG cancel intents from LfgIntentStore (main-thread-only store).
// Runs on OnTickDrain — safe to read the store without locking.
//
// C++ adapter canonical field set (must match LfgCancelArgs in tool_schemas.py):
//   args.value("max", 64)   — optional u32 cap on how many cancel guids to drain
//
// Response shape consumed by harness.rs::lfg_cancel:
//   { "cancelled": [<u64 guid_low>, ...] }

#include "Adapters/LfgCancelAdapter.h"
#include "LfgIntentStore.h"

namespace HarnessBridge::Adapters
{
    DispatchResult LfgCancel(nlohmann::json const& args)
    {
        DispatchResult res;

        // `max` is optional; default 64.  No hard-required fields.
        std::size_t const max = static_cast<std::size_t>(
            args.value("max", 64));

        auto guids = DrainLfgCancels(max);

        // Serialize into the shape the Rust slice expects:
        //   result.cancelled[] — array of plain u64 guid_low values.
        nlohmann::json cancelled = nlohmann::json::array();
        for (uint64_t guid : guids)
            cancelled.push_back(guid);

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"cancelled", std::move(cancelled)}};
        return res;
    }

} // namespace HarnessBridge::Adapters
