#include "Adapters/BotSetRoleAdapter.h"

#include "Group.h"
#include "ObjectAccessor.h"
#include "Opcodes.h"
#include "Player.h"
#include "Script/Playerbots.h"
#include "WorldPacket.h"

// LFG role bitmask constants (from LFGMgr.h, src/server/game/DungeonFinding/).
// Numeric values confirmed via live probe P5 after Stage 1 deploy.
#define HARNESS_ROLE_TANK    0x02
#define HARNESS_ROLE_HEALER  0x04
#define HARNESS_ROLE_DAMAGE  0x08
#define HARNESS_ROLE_NONE    0x00

namespace HarnessBridge::Adapters
{
    DispatchResult BotSetRole(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid") || !args.contains("role"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.set_role: bot_guid + role required";
            return res;
        }
        uint64_t bot_low   = args["bot_guid"].get<uint64_t>();
        std::string role   = args["role"].get<std::string>();

        bool valid_role =
            role == "tank" || role == "healer" || role == "dps" ||
            role == "leader" || role == "none";
        if (!valid_role)
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message =
                "bot.set_role: role must be one of tank|healer|dps|leader|none; got \"" + role + "\"";
            return res;
        }

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

        if (role == "leader")
        {
            Group* g = p->GetGroup();
            if (!g)
            {
                res.outcome = DispatchResult::Outcome::ExecutorFailed;
                res.error_message = "bot.set_role: bot is not in a group; cannot transfer leadership";
                return res;
            }
            // Mirror PlayerbotOperations.h:215-253 GroupSetLeaderOperation.
            g->ChangeLeader(p->GetGUID());

            res.outcome = DispatchResult::Outcome::Ok;
            res.result_json = {
                {"role_set",   "leader"},
                {"roles_mask", 0},
            };
            return res;
        }

        // LFG role path — mirror LfgActions.cpp:177-194.
        uint8 newRoles = HARNESS_ROLE_NONE;
        if      (role == "tank")   newRoles = HARNESS_ROLE_TANK;
        else if (role == "healer") newRoles = HARNESS_ROLE_HEALER;
        else if (role == "dps")    newRoles = HARNESS_ROLE_DAMAGE;
        // "none" stays 0x00.

        WorldPacket* data = new WorldPacket(CMSG_LFG_SET_ROLES);
        *data << (uint8)newRoles;
        p->GetSession()->QueuePacket(data);

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"role_set",   role},
            {"roles_mask", static_cast<int>(newRoles)},
        };
        return res;
    }
}
