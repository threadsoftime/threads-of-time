#include "ConsoleCaptureStore.h"

#include <algorithm>
#include <utility>
#include <vector>

namespace HarnessBridge
{
    void ConsoleCaptureStore::Insert(std::string const& request_id)
    {
        // Idempotent: if entry exists, leave it untouched.
        if (_captures.find(request_id) != _captures.end())
            return;
        _captures.emplace(request_id, Capture{});
        _seq_by_id.emplace(request_id, _next_seq++);
    }

    void ConsoleCaptureStore::Append(std::string const& request_id,
                                     std::string_view text)
    {
        auto it = _captures.find(request_id);
        if (it == _captures.end())
            return;
        Capture& cap = it->second;
        if (cap.buffer.size() >= kCaptureMaxBufferBytes)
        {
            cap.truncated = true;
            return;
        }
        std::size_t remaining = kCaptureMaxBufferBytes - cap.buffer.size();
        if (text.size() > remaining)
        {
            cap.buffer.append(text.substr(0, remaining));
            cap.truncated = true;
        }
        else
        {
            cap.buffer.append(text);
        }
    }

    void ConsoleCaptureStore::MarkDone(std::string const& request_id, bool success)
    {
        auto it = _captures.find(request_id);
        if (it == _captures.end())
            return;
        it->second.done = true;
        it->second.success = success;
        it->second.done_at = std::chrono::steady_clock::now();
    }

    std::optional<Capture> ConsoleCaptureStore::Read(std::string const& request_id) const
    {
        auto it = _captures.find(request_id);
        if (it == _captures.end())
            return std::nullopt;
        return it->second;
    }

    void ConsoleCaptureStore::EvictExpired(std::chrono::steady_clock::time_point now)
    {
        // 1. TTL sweep on done captures.
        for (auto it = _captures.begin(); it != _captures.end(); )
        {
            if (it->second.done && (now - it->second.done_at) > kCaptureEvictionTtl)
            {
                _seq_by_id.erase(it->first);
                it = _captures.erase(it);
            }
            else
            {
                ++it;
            }
        }

        // 2. Hard cap: if we still have too many entries, evict the
        //    oldest ones by insertion order regardless of done status.
        if (_captures.size() <= kCaptureMaxEntries)
            return;

        std::vector<std::pair<std::uint64_t, std::string>> by_age;
        by_age.reserve(_captures.size());
        for (auto const& [id, seq] : _seq_by_id)
            by_age.emplace_back(seq, id);
        std::sort(by_age.begin(), by_age.end());

        std::size_t to_evict = _captures.size() - kCaptureMaxEntries;
        for (std::size_t i = 0; i < to_evict && i < by_age.size(); ++i)
        {
            _captures.erase(by_age[i].second);
            _seq_by_id.erase(by_age[i].second);
        }
    }

    ConsoleCaptureStore& ConsoleCaptureStore::Instance()
    {
        static ConsoleCaptureStore inst;
        return inst;
    }
}
