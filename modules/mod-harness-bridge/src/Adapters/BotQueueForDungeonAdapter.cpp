#include "Adapters/BotQueueForDungeonAdapter.h"

#include "LFGMgr.h"
#include "LfgIntentStore.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "Script/Playerbots.h"

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
        if (!args.contains("bot_guid") || !args.contains("dungeon_id"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.queue_for_dungeon: bot_guid + dungeon_id required";
            return res;
        }
        uint64_t bot_low   = args["bot_guid"].get<uint64_t>();
        int dungeon_id_arg = args["dungeon_id"].get<int>();
        // roles_mask is optional: absent or 0 → auto-detect from bot's talent spec.
        int roles_mask_arg = args.contains("roles_mask") ? args["roles_mask"].get<int>() : 0;

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

        if (p->GetGroup())
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot.queue_for_dungeon: bot is already in a group";
            return res;
        }

        if (p->isDead())
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot.queue_for_dungeon: bot is dead";
            return res;
        }

        // Auto-detect role if roles_mask == 0 (absent or explicitly 0).
        //
        // Use bySpec=true to derive the role from the bot's dominant talent tree
        // rather than the runtime combat-strategy set.  The bySpec=false default
        // checks ContainsStrategy(STRATEGY_TYPE_TANK/HEAL), which reflects which
        // strategy was loaded when the bot was initialised — not necessarily the
        // talent spec.  In practice most bots' strategies are initialised to DPS
        // (e.g. a Holy-spec paladin gets "dps" strategy at level ≤25 before the
        // strategy engine re-evaluates), so bySpec=false makes every bot appear
        // as DPS regardless of spec.  bySpec=true goes straight to
        // AiFactory::GetPlayerSpecTab, which reads the live talent map and
        // returns the correct tab regardless of loaded strategies.
        uint8 roleMask = static_cast<uint8>(roles_mask_arg);
        if (roleMask == 0)
        {
            if (PlayerbotAI::IsTank(p, /*bySpec=*/true))
                roleMask = HARNESS_ROLE_TANK;
            else if (PlayerbotAI::IsHeal(p, /*bySpec=*/true))
                roleMask = HARNESS_ROLE_HEALER;
            else
                roleMask = HARNESS_ROLE_DAMAGE;
        }

        // Resolve dungeon ID: 0 from caller = use random sentinel.
        uint32 dungeonId = (dungeon_id_arg == 0)
            ? HARNESS_LFG_RANDOM_DUNGEON
            : static_cast<uint32>(dungeon_id_arg);

        // Reroute (LFG Inc 1, Stage 4): record the bot's LFG intent for the Rust
        // slice to drain via obs.lfg_pending, instead of constructing CMSG_LFG_JOIN
        // into native LFGMgr. LfgVetoScript does NOT veto bots, so the native path
        // would still reach LFGQueue matching — recording the intent keeps native
        // matching at zero callers. Mirrors the LfgActions.cpp force-queue reroute.
        HarnessBridge::RecordLfgIntent({
            bot_low,
            static_cast<uint8_t>(p->GetTeamId()),
            static_cast<uint32_t>(roleMask),
            std::vector<uint32_t>{dungeonId},
            /*comment=*/ "",
            /*is_bot=*/true
        });

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"queued",     true},
            {"via",        "slice_intent"},
            {"dungeon_id", static_cast<int>(dungeonId)},
            {"roles_mask", static_cast<int>(roleMask)},
        };
        return res;
    }
}
