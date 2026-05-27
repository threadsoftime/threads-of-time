#include "Adapters/BotSetGoalAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"
#include "Script/Playerbots.h"       // GET_PLAYERBOT_AI macro + PlayerbotAI forward
// Reuse existing Tools/ToolCatalog + Tools/ToolExecutors from mod-playerbots:
#include "Bot/LlmAgent/Tools/ToolCatalog.h"
#include "Bot/LlmAgent/Tools/ToolExecutors.h"
#include "Bot/LlmAgent/Tools/ToolValidators.h"
#include "Bot/LlmAgent/Tools/InteractionContext.h"  // SnapshotInteractionContext
#include "Bot/LlmAgent/Schemas/Goal.h"

namespace HarnessBridge::Adapters
{
    DispatchResult BotSetGoal(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid") || !args.contains("goal"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.set_goal: bot_guid + goal required";
            return res;
        }
        uint64_t target = args["bot_guid"].get<uint64_t>();
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

        // Parse goal JSON via the existing ParseAndValidate.
        auto parsed = ParseAndValidate(args["goal"].dump());
        if (std::holds_alternative<ParseError>(parsed))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "goal parse: " + std::get<ParseError>(parsed).message;
            return res;
        }
        ParsedGoal goal = std::get<ParsedGoal>(parsed);

        // Build SetGoalCall + delegate to existing validator/executor.
        SetGoalCall call{goal};
        InteractionContext ctx = SnapshotInteractionContext(ai);

        // Validator (free overload: Validate(call, ctx))
        ValidationResult vresult = Validate(call, ctx);
        if (!vresult.accepted)
        {
            res.outcome = DispatchResult::Outcome::ValidatorRejected;
            res.error_message = vresult.reject_reason;
            return res;
        }
        // Executor (LlmAgentTools::ApplySetGoal(call, botAI))
        bool ok = LlmAgentTools::ApplySetGoal(call, ai);
        if (!ok)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "ApplySetGoal returned false";
            return res;
        }
        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"applied", true}};
        return res;
    }
}
