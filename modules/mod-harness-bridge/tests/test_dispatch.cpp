// DOCTEST_CONFIG_IMPLEMENT_WITH_MAIN is defined in test_config.cpp (one TU only).
#include <doctest/doctest.h>

// We test DispatchQueue in isolation, mocking out the AC-side bits.
// Re-declare the minimum types so we don't pull in AC headers.

#include <future>
#include <memory>
#include <mutex>
#include <queue>
#include <string>
#include <vector>

namespace HarnessBridge
{
    struct DispatchResult { int outcome = 0; };
    struct PendingWork
    {
        std::string tool_name;
        std::promise<DispatchResult> result_promise;
    };

    class DispatchQueue
    {
    public:
        explicit DispatchQueue(std::size_t cap) : _cap(cap) {}
        bool TryPush(std::unique_ptr<PendingWork> w)
        {
            std::lock_guard<std::mutex> lk(_mu);
            if (_q.size() >= _cap) return false;
            _q.push(std::move(w));
            return true;
        }
        std::size_t DrainUpTo(std::size_t max,
                              std::vector<std::unique_ptr<PendingWork>>& out)
        {
            std::lock_guard<std::mutex> lk(_mu);
            std::size_t taken = 0;
            while (taken < max && !_q.empty())
            {
                out.push_back(std::move(_q.front()));
                _q.pop();
                ++taken;
            }
            return taken;
        }
        std::size_t Size() const
        {
            std::lock_guard<std::mutex> lk(_mu);
            return _q.size();
        }
    private:
        mutable std::mutex _mu;
        std::queue<std::unique_ptr<PendingWork>> _q;
        std::size_t _cap;
    };
}

TEST_CASE("DispatchQueue accepts up to cap then rejects") {
    HarnessBridge::DispatchQueue q(3);
    CHECK(q.TryPush(std::make_unique<HarnessBridge::PendingWork>()));
    CHECK(q.TryPush(std::make_unique<HarnessBridge::PendingWork>()));
    CHECK(q.TryPush(std::make_unique<HarnessBridge::PendingWork>()));
    CHECK_FALSE(q.TryPush(std::make_unique<HarnessBridge::PendingWork>()));
    CHECK(q.Size() == 3);
}

TEST_CASE("DispatchQueue DrainUpTo respects max and FIFO") {
    HarnessBridge::DispatchQueue q(10);
    for (int i = 0; i < 5; ++i)
    {
        auto w = std::make_unique<HarnessBridge::PendingWork>();
        w->tool_name = "tool_" + std::to_string(i);
        q.TryPush(std::move(w));
    }
    std::vector<std::unique_ptr<HarnessBridge::PendingWork>> out;
    auto taken = q.DrainUpTo(3, out);
    CHECK(taken == 3);
    CHECK(out.size() == 3);
    CHECK(out[0]->tool_name == "tool_0");
    CHECK(out[2]->tool_name == "tool_2");
    CHECK(q.Size() == 2);
}
