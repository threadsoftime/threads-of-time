#include "Adapters/BotFollowAdapter.h"

#include "Group.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "Script/Playerbots.h"

namespace HarnessBridge::Adapters
{
    DispatchResult BotFollow(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid") || !args.contains("leader_guid"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.follow: bot_guid + leader_guid required";
            return res;
        }
        uint64_t bot_low    = args["bot_guid"].get<uint64_t>();
        uint64_t leader_low = args["leader_guid"].get<uint64_t>();

        Player* p = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(bot_low));
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

        Player* leader = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(leader_low));
        if (!leader || !leader->IsInWorld())
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "leader not online";
            return res;
        }

        // Group-conflict guard: the MeleeFormation reads "group leader" which
        // resolves via PlayerbotAI::GetGroupLeader() → group leader if grouped,
        // else master. If the bot is in a group whose leader is NOT the
        // requested leader, SetMaster will be ignored.
        if (Group* g = p->GetGroup())
        {
            if (g->GetLeaderGUID() != leader->GetGUID())
            {
                res.outcome = DispatchResult::Outcome::ValidatorRejected;
                res.error_message =
                    "bot is in a group whose leader is not the requested leader; "
                    "the follow formation resolves to GetGroupLeader "
                    "(PlayerbotAI.cpp:4435-4443). Leave the group first or use the "
                    "current group leader.";
                return res;
            }
        }

        // Set master, then mirror the chat-shortcut "come" idiom verbatim
        // (ChatShortcutActions.cpp:52-53).
        ai->SetMaster(leader);
        // Clear RPG state machine so the 3.0f-relevance "new rpg wander npc"
        // action stops overriding the 1.0f-relevance follow action.
        // NewRpgStrategy registers wander npc at relevance 3.0f
        // (NewRpgStrategy.cpp:47-49); FollowMasterStrategy registers follow
        // at 1.0f (FollowMasterStrategy.cpp:11). Without this call the bot
        // continues walking toward its NPC target instead of following.
        ai->rpgInfo.ChangeToIdle();
        ai->ChangeStrategy("+follow,-passive,-grind,-move from group", BOT_STATE_NON_COMBAT);
        ai->ChangeStrategy("+follow,-passive,-grind,-move from group", BOT_STATE_COMBAT);

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"applied",     true},
            {"leader_guid", leader_low},
            {"leader_name", leader->GetName()},
        };
        return res;
    }
}
