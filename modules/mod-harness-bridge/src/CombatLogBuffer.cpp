#include "CombatLogBuffer.h"

namespace HarnessBridge
{
    void CombatLogBuffer::Append(CombatEvent ev)
    {
        std::lock_guard<std::mutex> lk(_mu);
        auto& dq = _by_target[ev.target_guid];
        dq.push_back(std::move(ev));
        if (dq.size() > kPerTargetCap)
            dq.pop_front();
    }

    std::vector<CombatEvent> CombatLogBuffer::Slice(
        uint64 target_guid, uint64 since_ts_ms, std::size_t limit) const
    {
        std::lock_guard<std::mutex> lk(_mu);
        std::vector<CombatEvent> out;
        auto it = _by_target.find(target_guid);
        if (it == _by_target.end())
            return out;

        out.reserve(std::min(limit, it->second.size()));
        for (auto rit = it->second.rbegin(); rit != it->second.rend() && out.size() < limit; ++rit)
        {
            if (rit->ts_ms < since_ts_ms)
                break;
            out.push_back(*rit);
        }
        std::reverse(out.begin(), out.end());
        return out;
    }

    CombatLogBuffer& Buffer()
    {
        static CombatLogBuffer b;
        return b;
    }
}
