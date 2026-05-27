#include "Adapters/BotAcceptInviteAdapter.h"

#include "Group.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "Script/Playerbots.h"

namespace HarnessBridge::Adapters
{
    DispatchResult BotAcceptInvite(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid") || !args["bot_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.accept_invite: bot_guid (int) required";
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

        if (p->GetGroupInvite() == nullptr)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot.accept_invite: no pending group invitation";
            return res;
        }

        // Mirror AcceptInvitationAction.cpp:41-44.
        // WotLK HandleGroupAcceptOpcode reads uint32 roles_mask from packet body;
        // a default-constructed empty WorldPacket throws "ByteBuffer size 0".
        // V1.5 probe T17/P3 surfaced this. Push roles_mask=0 before calling.
        WorldPacket pkt;
        uint32 roles_mask = 0;
        pkt << roles_mask;
        p->GetSession()->HandleGroupAcceptOpcode(pkt);

        Group* g = p->GetGroup();
        uint64_t group_guid_low = g ? g->GetGUID().GetCounter() : 0;
        bool is_raid = g ? g->isRaidGroup() : false;

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"accepted",   true},
            {"group_guid", group_guid_low},
            {"group_type", is_raid ? "raid" : "party"},
        };
        return res;
    }
}
