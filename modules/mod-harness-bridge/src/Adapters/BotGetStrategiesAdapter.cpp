#include "Adapters/BotGetStrategiesAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"
#include "Script/Playerbots.h"
#include "Bot/Engine/AiObjectContext.h"   // GetSupportedStrategies()

#include <set>
#include <string>
#include <vector>

namespace HarnessBridge::Adapters
{
    static bool ResolveBucketsForGet(std::string const& bot_state,
                                      std::vector<BotState>& out)
    {
        out.clear();
        if (bot_state == "all")
        {
            out.push_back(BOT_STATE_COMBAT);
            out.push_back(BOT_STATE_NON_COMBAT);
            out.push_back(BOT_STATE_DEAD);
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

    DispatchResult BotGetStrategies(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.get_strategies: bot_guid required";
            return res;
        }
        uint64_t target = args["bot_guid"].get<uint64_t>();
        std::string bot_state = args.value("bot_state", std::string("all"));

        std::vector<BotState> buckets;
        if (!ResolveBucketsForGet(bot_state, buckets))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message =
                "bot.get_strategies: bot_state must be one of "
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

        nlohmann::json active = nlohmann::json::object();
        for (BotState s : buckets)
        {
            std::vector<std::string> names = ai->GetStrategies(s);
            active[BucketName(s)] = names;
        }

        // Available strategies (global to bot's AiObjectContext; built per class).
        std::set<std::string> available_set =
            ai->GetAiObjectContext()->GetSupportedStrategies();
        std::vector<std::string> available(available_set.begin(), available_set.end());

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"active",    active},
            {"available", available},
        };
        return res;
    }
}
