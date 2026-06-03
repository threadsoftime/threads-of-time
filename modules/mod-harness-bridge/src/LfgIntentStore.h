// LfgIntentStore — main-thread-only intent queue for the strangler-fig LFG slice
//
// THREAD SAFETY INVARIANT:
//   This store is accessed ONLY on the world/main thread.  There are exactly
//   two writers and one reader, all on the same thread:
//     - Writer A: LfgVetoScript::OnPlayerCanJoinLfg (fires on CMSG_LFG_JOIN,
//       handled PROCESS_THREADUNSAFE — i.e. on the world update thread).
//     - Writer B: mod-agenticbots Stage-4 bot-queue (not yet wired; will call
//       RecordLfgIntent directly after obtaining its Player* on the tick).
//     - Reader:   ObsLfgPendingAdapter via DrainLfgIntents (runs in OnTickDrain,
//       also on the world update thread).
//   No mutex is used.  Adding one would be a bug — it would create false safety
//   that invites cross-thread use.  Mirror the same note in HarnessBridgeDispatch.h.
//
// PUBLIC SEAM:
//   RecordLfgIntent and DrainLfgIntents are the only two entry points.
//   Other modules (mod-agenticbots Stage 4) may #include this header and call
//   RecordLfgIntent without pulling in the full HarnessBridge stack.

#pragma once

#include <cstdint>
#include <cstddef>
#include <string>
#include <vector>

namespace HarnessBridge
{
    struct LfgIntent
    {
        uint64_t              guid_low;
        uint8_t               team_id;      // 0=Alliance, 1=Horde (Player::GetTeamId())
        uint32_t              roles;        // LFG role bitmask (ROLES_MASK_*)
        std::vector<uint32_t> dungeon_ids;  // contents of lfg::LfgDungeonSet for this join
        std::string           comment;
        bool                  is_bot;       // false for real players (veto hook);
                                            // true once Stage-4 bot-queue wires in
    };

    // Record one intent — called by the veto hook (real players) and by
    // the Stage-4 bot-queue (bots migrated off native LFG).  Both callers
    // run on the world thread.  Copies only POD + value types from Player*;
    // the Player pointer itself NEVER enters the store.
    void RecordLfgIntent(LfgIntent intent);

    // Pop and return up to `max` intents from the front of the queue.
    // At-most-once semantics: returned intents are removed from the store.
    // Called by ObsLfgPendingAdapter on the world tick.
    std::vector<LfgIntent> DrainLfgIntents(std::size_t max);

    // Record one cancel intent — called by HandleLfgLeaveOpcode (world thread).
    // Stores only the POD guid_low; Player* is NEVER held here.
    void RecordLfgCancel(uint64_t guid_low);

    // Pop and return up to `max` cancel guids from the front of the queue.
    // At-most-once semantics: returned entries are removed.
    // Called by LfgCancelAdapter::LfgCancel on the world tick.
    std::vector<uint64_t> DrainLfgCancels(std::size_t max);

} // namespace HarnessBridge
