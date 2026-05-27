#include "Adapters/ObsGetStateAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"
// mod-playerbots' Tier0 digest builder. `BuildDigest(PlayerbotAI*)` is
// the convenience that wraps SnapshotBot + BuildDigestJson. Both are
// free functions in the global namespace.
#include "Bot/LlmAgent/Tiers/Tier0_StateDigest.h"
// GET_PLAYERBOT_AI(Player*) → PlayerbotAI*. Lives in mod-playerbots.
#include "Script/Playerbots.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetState(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "obs.get_state: target_guid (int) required";
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
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "player " + std::to_string(target) + " is not a bot — obs.get_state requires bot AI for Tier0 digest";
            return res;
        }

        // BuildDigest = SnapshotBot + BuildDigestJson. Returns the same
        // JSON shape the LLM agent sees (kb_6c4b36a9).
        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = BuildDigest(ai);
        return res;
    }
}
