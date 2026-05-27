#include "Adapters/BotQueueForDungeonAdapter.h"

#include "LFGMgr.h"
#include "ObjectAccessor.h"
#include "Opcodes.h"
#include "Player.h"
#include "Script/Playerbots.h"
#include "WorldPacket.h"

// LFG role bitmask constants — must match BotSetRoleAdapter.cpp defines.
// Numeric values confirmed via probe P5 after Stage 1 deploy.
#define HARNESS_ROLE_TANK    0x02
#define HARNESS_ROLE_HEALER  0x04
#define HARNESS_ROLE_DAMAGE  0x08
#define HARNESS_ROLE_NONE    0x00

// Random-dungeon sentinel ID used by the LFG system.
// Pass-through: server resolves the level-appropriate entry.
#define HARNESS_LFG_RANDOM_DUNGEON 0xFFFFFFFF

namespace HarnessBridge::Adapters
{
    DispatchResult BotQueueForDungeon(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid") || !args.contains("dungeon_id") || !args.contains("roles_mask"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.queue_for_dungeon: bot_guid + dungeon_id + roles_mask required";
            return res;
        }
        uint64_t bot_low   = args["bot_guid"].get<uint64_t>();
        int dungeon_id_arg = args["dungeon_id"].get<int>();
        int roles_mask_arg = args["roles_mask"].get<int>();

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

        if (p->InBattleground() || p->InBattlegroundQueue())
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot.queue_for_dungeon: bot is in a battleground or battleground queue";
            return res;
        }

        if (sLFGMgr->GetState(p->GetGUID()) != lfg::LFG_STATE_NONE)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot.queue_for_dungeon: bot is already in an LFG queue or instance";
            return res;
        }

        // Auto-detect role if roles_mask == 0.
        uint8 roleMask = static_cast<uint8>(roles_mask_arg);
        if (roleMask == 0)
        {
            if (ai->IsTank(p))
                roleMask = HARNESS_ROLE_TANK;
            else if (ai->IsHeal(p))
                roleMask = HARNESS_ROLE_HEALER;
            else
                roleMask = HARNESS_ROLE_DAMAGE;
        }

        // Resolve dungeon ID: 0 from caller = use random sentinel.
        uint32 dungeonId = (dungeon_id_arg == 0)
            ? HARNESS_LFG_RANDOM_DUNGEON
            : static_cast<uint32>(dungeon_id_arg);

        // Mirror LfgActions.cpp:157-168.
        std::list<uint32> list = {dungeonId};
        WorldPacket* data = new WorldPacket(CMSG_LFG_JOIN);
        *data << (uint32)roleMask;
        *data << (bool)false << (bool)false;
        *data << (uint8)(list.size());
        for (uint32 d : list)
            *data << (uint32)d;
        *data << (uint8)3 << (uint8)0 << (uint8)0 << (uint8)0;
        *data << std::string("0");   // gear score placeholder
        p->GetSession()->QueuePacket(data);

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"queued",     true},
            {"dungeon_id", static_cast<int>(dungeonId)},
            {"roles_mask", static_cast<int>(roleMask)},
            {"lfg_state",  "queued"},
        };
        return res;
    }
}
