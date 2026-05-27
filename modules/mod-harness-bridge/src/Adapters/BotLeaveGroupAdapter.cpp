#include "Adapters/BotLeaveGroupAdapter.h"

#include "Group.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "Script/Playerbots.h"
#include "WorldPacket.h"
#include "Opcodes.h"

namespace HarnessBridge::Adapters
{
    DispatchResult BotLeaveGroup(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid") || !args["bot_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.leave_group: bot_guid (int) required";
            return res;
        }
        uint64_t bot_low = args["bot_guid"].get<uint64_t>();

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

        Group* g = p->GetGroup();
        if (!g)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot.leave_group: bot is not in a group";
            return res;
        }

        // Capture leader status before queuing the packet (packet disbands group
        // server-side when leader leaves; g pointer may be invalid after).
        bool was_leader = (g->GetLeaderGUID() == p->GetGUID());

        // Mirror PlayerbotAI::LeaveOrDisbandGroup() lines 909-916.
        // CMSG_GROUP_DISBAND is handled server-side as leave-or-disband.
        WorldPacket* data = new WorldPacket(CMSG_GROUP_DISBAND);
        p->GetSession()->QueuePacket(data);

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"left",       true},
            {"was_leader", was_leader},
        };
        return res;
    }
}
