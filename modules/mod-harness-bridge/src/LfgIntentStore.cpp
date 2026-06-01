// LfgIntentStore.cpp — translation-unit-local intent queue
//
// See LfgIntentStore.h for the thread-safety invariant.  No synchronisation
// primitives here — this is by design, not an omission.

#include "LfgIntentStore.h"

#include <algorithm>

namespace HarnessBridge
{
    namespace
    {
        // The sole instance of the queue.  Static storage, main-thread-only.
        // No cap is enforced here because the veto hook fires at human-speed
        // (one real player click at a time); the drain on the tick ensures the
        // queue stays near-empty in steady state.  A pathological flood of
        // rapid joins would grow this vector; acceptable for Inc-1 scope where
        // population is small.
        static std::vector<LfgIntent> gIntentQueue;
    }

    void RecordLfgIntent(LfgIntent intent)
    {
        gIntentQueue.push_back(std::move(intent));
    }

    std::vector<LfgIntent> DrainLfgIntents(std::size_t max)
    {
        if (gIntentQueue.empty() || max == 0)
            return {};

        std::size_t const take = std::min(max, gIntentQueue.size());

        // Move `take` entries from the front into the result.
        std::vector<LfgIntent> result;
        result.reserve(take);
        auto first = gIntentQueue.begin();
        auto last  = first + static_cast<std::ptrdiff_t>(take);
        for (auto it = first; it != last; ++it)
            result.push_back(std::move(*it));

        gIntentQueue.erase(first, last);
        return result;
    }

} // namespace HarnessBridge
