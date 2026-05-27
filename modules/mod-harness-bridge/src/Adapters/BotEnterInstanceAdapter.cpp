#include "Adapters/BotEnterInstanceAdapter.h"

#include "LFGMgr.h"
#include "ObjectAccessor.h"
#include "Opcodes.h"
#include "Player.h"
#include "Script/Playerbots.h"
#include "WorldPacket.h"

namespace HarnessBridge::Adapters
{
    DispatchResult BotEnterInstance(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid") || !args.contains("mode"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.enter_instance: bot_guid + mode required";
            return res;
        }
        uint64_t bot_low   = args["bot_guid"].get<uint64_t>();
        std::string mode   = args["mode"].get<std::string>();

        if (mode != "lfg" && mode != "direct")
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message =
                "bot.enter_instance: mode must be \"lfg\" or \"direct\"; got \"" + mode + "\"";
            return res;
        }

        int map_id_arg = args.value("map_id", -1);
        if (mode == "direct" && map_id_arg < 0)
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.enter_instance: map_id >= 0 required when mode=\"direct\"";
            return res;
        }

        float x           = args.value("x", 0.0f);
        float y           = args.value("y", 0.0f);
        float z           = args.value("z", 0.0f);
        float orientation = args.value("orientation", 0.0f);

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

        if (p->isDead())
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot.enter_instance: bot is dead";
            return res;
        }

        if (p->IsBeingTeleported())
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot.enter_instance: bot is already being teleported";
            return res;
        }

        int result_map_id = map_id_arg;

        if (mode == "lfg")
        {
            // V1.5 probe T17/P8 surfaced: previous version reported false-positive
            // success when no LFG match exists. Gate on LFG_STATE_DUNGEON.
            lfg::LfgState lfg_state = sLFGMgr->GetState(p->GetGUID());
            if (lfg_state != lfg::LFG_STATE_DUNGEON)
            {
                res.outcome = DispatchResult::Outcome::ExecutorFailed;
                res.error_message =
                    "bot.enter_instance: bot is not in an LFG-matched dungeon (state=" +
                    std::to_string(static_cast<int>(lfg_state)) + ")";
                return res;
            }
            // Mirror LfgTeleportAction::Execute lines 286-305: out=false teleports INTO dungeon.
            WorldPacket* packet = new WorldPacket(CMSG_LFG_TELEPORT);
            *packet << (bool)false;
            p->GetSession()->QueuePacket(packet);
            // map_id is not yet known at queue time (async); report -1.
            result_map_id = -1;
        }
        else
        {
            // mode == "direct" — pattern from GmTeleportAdapter.cpp.
            uint32 mapId = static_cast<uint32>(map_id_arg);
            bool ok = p->TeleportTo(mapId, x, y, z, orientation);
            if (!ok)
            {
                res.outcome = DispatchResult::Outcome::ExecutorFailed;
                res.error_message = "bot.enter_instance: TeleportTo returned false";
                return res;
            }
            result_map_id = static_cast<int>(mapId);
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"teleported", true},
            {"mode",       mode},
            {"map_id",     result_map_id},
        };
        return res;
    }
}
