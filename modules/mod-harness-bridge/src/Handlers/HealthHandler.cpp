#include "Handlers/HealthHandler.h"

#include "Bot/LlmAgent/Vendor/httplib.h"
#include "Bot/LlmAgent/Vendor/nlohmann_json.hpp"

#include <chrono>

namespace HarnessBridge
{
    void RegisterHealthHandler(httplib::Server& server)
    {
        server.Get("/health", [](httplib::Request const& /*req*/, httplib::Response& res) {
            auto now_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::system_clock::now().time_since_epoch()).count();
            nlohmann::json body = {
                {"ok",        true},
                {"service",   "harness-bridge"},
                {"ts_ms",     now_ms},
            };
            res.set_content(body.dump(), "application/json");
            res.status = 200;
        });
    }
}
