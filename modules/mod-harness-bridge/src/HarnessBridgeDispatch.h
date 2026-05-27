#ifndef MOD_HARNESS_BRIDGE_DISPATCH_H
#define MOD_HARNESS_BRIDGE_DISPATCH_H

#include "Common.h"
#include "Bot/LlmAgent/Vendor/nlohmann_json.hpp"

#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <future>
#include <mutex>
#include <optional>
#include <queue>
#include <string>
#include <unordered_map>

namespace HarnessBridge
{
    // The result the executor produces for the handler thread to serialize.
    struct DispatchResult
    {
        enum class Outcome
        {
            Ok,                  // executor returned true
            ValidatorRejected,   // validator pre-flight failed
            ExecutorFailed,      // executor returned false
            ExecutorThrew,       // std::exception caught in main-thread drain
            UnknownTool,         // no adapter matched
            BadArgs,             // adapter rejected JSON shape
            QueueFull,           // never sent to main thread
            Timeout,             // handler-thread future timed out
        };

        Outcome      outcome = Outcome::Ok;
        nlohmann::json result_json;     // executor output; null on failure
        std::string  error_message;     // human-readable on non-Ok
        uint64       tick_wait_ms = 0;  // time queued before drain
        uint64       executor_ms  = 0;  // time spent inside the executor
    };

    // A queued unit of work: parsed call + identity + result promise.
    struct PendingWork
    {
        std::string  request_id;
        std::string  identity;
        std::string  tool_name;
        nlohmann::json args_json;
        std::promise<DispatchResult> result_promise;
        // Defaults to construction time so a freshly-built PendingWork is
        // self-timestamping. DispatchHandler (Task 6) can overwrite this
        // explicitly if it wants the timestamp to be before JSON parse.
        std::chrono::steady_clock::time_point enqueued_at = std::chrono::steady_clock::now();
    };

    class DispatchQueue
    {
    public:
        explicit DispatchQueue(std::size_t cap) : _cap(cap) {}

        // Enqueue. Returns false if queue at cap.
        bool TryPush(std::unique_ptr<PendingWork> work);

        // Pop up to `max` items into `out`. Non-blocking. Called from
        // main thread inside OnUpdate.
        std::size_t DrainUpTo(std::size_t max, std::vector<std::unique_ptr<PendingWork>>& out);

        std::size_t Size() const;

    private:
        mutable std::mutex                                    _mu;
        std::queue<std::unique_ptr<PendingWork>>              _q;
        std::size_t                                           _cap;
    };

    // Singleton accessor for the running module's dispatch queue.
    DispatchQueue& Queue();

    // Drain entry called from WorldScript::OnUpdate. Runs the validator
    // and executor for each drained item, fulfills the promise.
    void OnTickDrain();
}

#endif
