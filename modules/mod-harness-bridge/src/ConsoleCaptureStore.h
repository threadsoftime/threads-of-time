#ifndef MOD_HARNESS_BRIDGE_CONSOLE_CAPTURE_STORE_H
#define MOD_HARNESS_BRIDGE_CONSOLE_CAPTURE_STORE_H

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_map>

namespace HarnessBridge
{
    // Buffer-cap and TTL constants. Public so tests can reference them
    // without re-declaring magic numbers.
    inline constexpr std::size_t kCaptureMaxBufferBytes = 64 * 1024;
    inline constexpr std::chrono::seconds kCaptureEvictionTtl{60};
    inline constexpr std::size_t kCaptureMaxEntries = 256;

    struct Capture
    {
        std::string buffer;
        bool        done       = false;
        bool        success    = false;
        bool        truncated  = false;
        std::chrono::steady_clock::time_point done_at{};
    };

    class ConsoleCaptureStore
    {
    public:
        // Lifecycle. All methods are main-thread only — no locking.
        void Insert(std::string const& request_id);
        void Append(std::string const& request_id, std::string_view text);
        void MarkDone(std::string const& request_id, bool success);
        std::optional<Capture> Read(std::string const& request_id) const;

        // Sweep: evict any Capture whose done_at is older than
        // now - kCaptureEvictionTtl. Also force-evicts the oldest entries
        // if the total exceeds kCaptureMaxEntries (defends against runaway
        // never-finished captures).
        void EvictExpired(std::chrono::steady_clock::time_point now);

        // Test-only: count current entries.
        std::size_t Size() const { return _captures.size(); }

        // Singleton accessor for production use.
        static ConsoleCaptureStore& Instance();

    private:
        // insertion_order tracks Insert() call order so eviction is FIFO.
        std::unordered_map<std::string, Capture> _captures;
        std::uint64_t _next_seq = 0;
        std::unordered_map<std::string, std::uint64_t> _seq_by_id;
    };
}

#endif
