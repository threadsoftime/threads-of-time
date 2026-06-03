// LfgVetoScript — PlayerScript that intercepts real-player LFG joins
//
// Fires on CMSG_LFG_JOIN (PROCESS_THREADUNSAFE = world update thread).
// Hook: OnPlayerCanJoinLfg (any-false-wins aggregation in ScriptMgr).
//
// Behaviour:
//   1. Bot players fall through to native LFG matching unchanged (Stage 4
//      will migrate them off native; until then, bots use the existing
//      BotQueueForDungeon / BotEnterInstance path or native LFG directly).
//   2. Real players are recorded into LfgIntentStore and vetoed from the
//      native LFGMgr::JoinLfg matching path (return false).
//   3. Async client feedback (best-effort): SendLfgJoinResult(LFG_JOIN_OK)
//      so the LFG eye icon appears to spin, immediately followed by
//      SendLfgUpdatePlayer(JOIN_QUEUE) to keep the UI in a queued state.
//      This mirrors LFGMgr::JoinLfg lines 885-886 for the successful-join
//      path.  The Rust matchmaker will later call lfg.form_group which sends
//      GROUP_FOUND + REMOVED_FROM_QUEUE (already handled by LfgFormGroupAdapter).
//
// NOTE on feedback accuracy: The eye-spin + JOIN_QUEUE feedback is believed
// correct based on LFGMgr.cpp:885-886 inspection, but has not been confirmed
// live prior to Deploy B.  Flag: empirical validation of eye-spin / eye-clear
// sequencing is an open item for Deploy B.  The veto + record path is robust
// regardless of whether the feedback packets are perfect.
//
// Thread safety: see LfgIntentStore.h — entire flow is main-thread-only.

#include "LfgIntentStore.h"
#include "LFGMgr.h"
#include "Player.h"
#include "WorldSession.h"
#include "ScriptMgr.h"
#include "Script/Playerbots.h"   // GET_PLAYERBOT_AI

namespace
{
    class LfgVetoScript : public PlayerScript
    {
    public:
        LfgVetoScript() : PlayerScript("LfgVetoScript") { }

        // Signature matches PlayerScript.h:619:
        //   virtual bool OnPlayerCanJoinLfg(Player*, uint8, std::set<uint32>&,
        //                                   const std::string&)
        // LfgDungeonSet = std::set<uint32> (LFG.h:114).
        // Aggregation: CALL_ENABLED_BOOLEAN_HOOKS with !script->..., so returning
        // false from any script suppresses native LFGMgr::JoinLfg (any-false-wins).
        bool OnPlayerCanJoinLfg(Player* player,
                                uint8 roles,
                                std::set<uint32>& dungeons,
                                const std::string& comment) override
        {
            if (!player)
                return true;    // safety guard; should not happen

            // Bots fall through to native LFG until Stage 4 migrates them.
            if (GET_PLAYERBOT_AI(player))
                return true;

            // Real player: record intent into the main-thread-local store.
            // Copy only POD + value types — Player* never escapes this frame.
            HarnessBridge::LfgIntent intent;
            intent.guid_low   = player->GetGUID().GetCounter();
            intent.team_id    = static_cast<uint8_t>(player->GetTeamId());
            intent.roles      = static_cast<uint32_t>(roles);
            intent.dungeon_ids.assign(dungeons.begin(), dungeons.end());
            intent.comment    = comment;
            intent.is_bot     = false;

            HarnessBridge::RecordLfgIntent(std::move(intent));

            // Async client feedback — best-effort (see file header note).
            // LFGMgr.cpp:885: player->GetSession()->SendLfgJoinResult(joinData)
            // LFGMgr.cpp:886: player->GetSession()->SendLfgUpdatePlayer(JOIN_QUEUE)
            if (WorldSession* sess = player->GetSession())
            {
                // LFG_JOIN_OK = 0 (LFGMgr.h:102).  Default-constructed
                // LfgJoinResultData uses LFG_JOIN_OK + LFG_ROLECHECK_DEFAULT.
                lfg::LfgJoinResultData joinOk;
                sess->SendLfgJoinResult(joinOk);

                // JOIN_QUEUE with the player's own selected dungeons + comment
                // mirrors what LFGMgr::JoinLfg sends on the successful path.
                lfg::LfgUpdateData joinQueue(
                    lfg::LFG_UPDATETYPE_JOIN_QUEUE,
                    dungeons,
                    comment);
                sess->SendLfgUpdatePlayer(joinQueue);
            }

            // Mirror the server-side state native LFGMgr::JoinLfg would have set,
            // so the player appears queued to GetState consumers AND the Inc-2 leave
            // seam (HandleLfgLeaveOpcode) fires on CMSG_LFG_LEAVE. We do NOT add the
            // player to any LFGQueue bucket — matchmaking is owned by the Rust slice —
            // so native matching stays bypassed; only the state flag is set.
            sLFGMgr->SetState(player->GetGUID(), lfg::LFG_STATE_QUEUED);

            // Return false: suppress native LFGMgr::JoinLfg matching.
            // The Rust matchmaker will handle the actual group formation.
            return false;
        }
    };

} // anonymous namespace

// Called from AddHarnessBridgeScripts() in HarnessBridge.cpp.
// The linker will GC this TU if AddLfgVetoScript is not called — do NOT
// add a standalone AddSC_ entry; route through the module's existing
// script-registration point.
void AddLfgVetoScript()
{
    new LfgVetoScript();
}
