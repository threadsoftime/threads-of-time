#include "Adapters/BotSetStrategyAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"
#include "Script/Playerbots.h"   // GET_PLAYERBOT_AI + PlayerbotAI forward

#include <string>
#include <vector>

namespace HarnessBridge::Adapters
{
    // Map bot_state string -> BotState enum value(s) to apply.
    // "all" fans to combat + non_combat (mirrors ChatShortcutActions.cpp:52-53).
    // "dead" maps to BOT_STATE_DEAD; explicit "combat" / "non_combat" target a single bucket.
    static bool ResolveBuckets(std::string const& bot_state, std::vector<BotState>& out)
    {
        out.clear();
        if (bot_state == "all")
        {
            out.push_back(BOT_STATE_COMBAT);
            out.push_back(BOT_STATE_NON_COMBAT);
            return true;
        }
        if (bot_state == "combat")     { out.push_back(BOT_STATE_COMBAT);     return true; }
        if (bot_state == "non_combat") { out.push_back(BOT_STATE_NON_COMBAT); return true; }
        if (bot_state == "dead")       { out.push_back(BOT_STATE_DEAD);       return true; }
        return false;
    }

    static char const* BucketName(BotState s)
    {
        switch (s)
        {
            case BOT_STATE_COMBAT:     return "combat";
            case BOT_STATE_NON_COMBAT: return "non_combat";
            case BOT_STATE_DEAD:       return "dead";
            default:                   return "unknown";
        }
    }

    DispatchResult BotSetStrategy(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid") || !args.contains("strategy"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.set_strategy: bot_guid + strategy required";
            return res;
        }
        uint64_t target = args["bot_guid"].get<uint64_t>();
        std::string strategy = args["strategy"].get<std::string>();
        std::string bot_state = args.value("bot_state", std::string("all"));

        std::vector<BotState> buckets;
        if (!ResolveBuckets(bot_state, buckets))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message =
                "bot.set_strategy: bot_state must be one of "
                "\"combat\", \"non_combat\", \"dead\", \"all\"; got \"" + bot_state + "\"";
            return res;
        }

        Player* p = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(target));
        if (!p)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot not online";
            return res;
        }
        PlayerbotAI* ai = GET_PLAYERBOT_AI(p);
        if (!ai)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "player is not a bot";
            return res;
        }

        // Apply and capture post-change strategy list per bucket.
        nlohmann::json buckets_json = nlohmann::json::object();
        for (BotState s : buckets)
        {
            ai->ChangeStrategy(strategy, s);
            std::vector<std::string> active = ai->GetStrategies(s);
            buckets_json[BucketName(s)] = active;
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"applied", true},
            {"buckets", buckets_json},
        };
        return res;
    }
}
