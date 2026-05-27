#include "Adapters/BotStopAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"
#include "Script/Playerbots.h"

namespace HarnessBridge::Adapters
{
    DispatchResult BotStop(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.stop: bot_guid required";
            return res;
        }
        uint64_t target = args["bot_guid"].get<uint64_t>();
        bool clear_combat = args.value("clear_combat", false);

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

        // Mirror the chat-shortcut "stay" idiom (ChatShortcutActions.cpp:122-123).
        ai->ChangeStrategy("+stay,-follow,-passive,-move from group", BOT_STATE_NON_COMBAT);
        ai->ChangeStrategy("+stay,-follow,-passive,-move from group", BOT_STATE_COMBAT);

        if (clear_combat)
        {
            p->CombatStop(true);
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"applied",        true},
            {"cleared_combat", clear_combat},
        };
        return res;
    }
}
