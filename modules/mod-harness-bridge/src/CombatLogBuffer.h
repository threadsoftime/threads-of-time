#ifndef MOD_HARNESS_BRIDGE_COMBAT_LOG_BUFFER_H
#define MOD_HARNESS_BRIDGE_COMBAT_LOG_BUFFER_H

#include "Common.h"

#include <chrono>
#include <deque>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

namespace HarnessBridge
{
    struct CombatEvent
    {
        uint64       ts_ms = 0;
        uint32       spell_id = 0;
        uint64       source_guid = 0;
        uint64       target_guid = 0;
        int32        amount = 0;
        std::string  kind;       // "damage", "heal", "absorb", "miss"
        std::string  school;     // "physical", "fire", "shadow", ...
        bool         is_crit = false;
    };

    // Per-target ring buffer (bounded to last N events per target).
    // Older entries evicted FIFO. Read access returns events with ts_ms
    // >= since.
    class CombatLogBuffer
    {
    public:
        static constexpr std::size_t kPerTargetCap = 500;  // ~30s of heavy combat

        void Append(CombatEvent ev);
        std::vector<CombatEvent> Slice(uint64 target_guid, uint64 since_ts_ms, std::size_t limit) const;

    private:
        mutable std::mutex _mu;
        std::unordered_map<uint64, std::deque<CombatEvent>> _by_target;
    };

    CombatLogBuffer& Buffer();
}

#endif
