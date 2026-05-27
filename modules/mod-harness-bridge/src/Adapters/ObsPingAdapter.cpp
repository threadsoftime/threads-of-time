#include "Adapters/ObsPingAdapter.h"

#include <chrono>

namespace HarnessBridge::Adapters
{
    DispatchResult ObsPing(nlohmann::json const& /*args*/)
    {
        DispatchResult res;
        res.outcome = DispatchResult::Outcome::Ok;
        auto now_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::system_clock::now().time_since_epoch()).count();
        res.result_json = {
            {"pong",  true},
            {"ts_ms", now_ms},
        };
        return res;
    }
}
