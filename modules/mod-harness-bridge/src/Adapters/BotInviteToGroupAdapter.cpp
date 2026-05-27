#include "Adapters/BotInviteToGroupAdapter.h"

#include "Group.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "Script/Playerbots.h"

namespace HarnessBridge::Adapters
{
    DispatchResult BotInviteToGroup(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid") || !args.contains("target_guid"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.invite_to_group: bot_guid + target_guid required";
            return res;
        }
        uint64_t bot_low    = args["bot_guid"].get<uint64_t>();
        uint64_t target_low = args["target_guid"].get<uint64_t>();

        if (bot_low == target_low)
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.invite_to_group: bot_guid and target_guid must differ";
            return res;
        }

        Player* inviter = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(bot_low));
        if (!inviter)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot not online";
            return res;
        }
        PlayerbotAI* ai = GET_PLAYERBOT_AI(inviter);
        if (!ai)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "player is not a bot";
            return res;
        }

        Player* target = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(target_low));
        if (!target || !target->IsInWorld())
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "target not online";
            return res;
        }

        if (target->GetGroup() != nullptr)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot.invite_to_group: target is already in a group";
            return res;
        }

        if (inviter->GetGroup() && inviter->GetGroup()->IsFull())
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot.invite_to_group: inviter's group is full";
            return res;
        }

        if (inviter->GetTeamId() != target->GetTeamId())
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot.invite_to_group: cross-faction invite not allowed";
            return res;
        }

        // Mirror InviteToGroupAction.cpp lines 37-43.
        WorldPacket p;
        uint32 roles_mask = 0;
        p << target->GetName();
        p << roles_mask;
        inviter->GetSession()->HandleGroupInviteOpcode(p);

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"invited",      true},
            {"target_guid",  target_low},
            {"target_name",  target->GetName()},
            {"inviter_name", inviter->GetName()},
        };
        return res;
    }
}
