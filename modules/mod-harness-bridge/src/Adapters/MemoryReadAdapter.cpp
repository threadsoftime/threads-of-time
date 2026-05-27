// SPDX-License-Identifier: GPL-2.0-or-later
// memory.read adapter — GET {HARNESS_MEMORY_URL}/v1/memory/{bot_guid}/episodes/{episode_id}
// Per design subspec §10.2 (EpisodeReadResponse shape). No side effects on the DB.
#include "Adapters/MemoryReadAdapter.h"

#include "Bot/LlmAgent/Vendor/httplib.h"
#include "Log.h"

#include <cstdlib>
#include <string>

namespace HarnessBridge::Adapters
{
    namespace
    {
        std::pair<std::string, int> MemoryBaseUrl()
        {
            const char* env = std::getenv("HARNESS_MEMORY_URL");
            std::string url = env ? env : "http://localhost:8090";

            auto schemeEnd = url.find("://");
            std::string authority = (schemeEnd == std::string::npos)
                ? url
                : url.substr(schemeEnd + 3);

            auto slashPos = authority.find('/');
            if (slashPos != std::string::npos)
                authority = authority.substr(0, slashPos);

            auto colonPos = authority.rfind(':');
            if (colonPos == std::string::npos)
                return {authority, 8090};

            std::string host = authority.substr(0, colonPos);
            int port = std::stoi(authority.substr(colonPos + 1));
            return {host, port};
        }
    }  // namespace

    DispatchResult MemoryRead(nlohmann::json const& args)
    {
        DispatchResult res;

        if (!args.contains("bot_guid") || !args.contains("episode_id"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "memory.read: bot_guid and episode_id required";
            return res;
        }

        std::string bot_guid  = args["bot_guid"].get<std::string>();
        int64_t     episode_id = args["episode_id"].get<int64_t>();

        auto [host, port] = MemoryBaseUrl();
        httplib::Client cli(host, port);
        cli.set_connection_timeout(5);
        cli.set_read_timeout(10);

        std::string path = "/v1/memory/" + bot_guid + "/episodes/" + std::to_string(episode_id);
        auto result = cli.Get(path.c_str());

        if (!result)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.read: HTTP call failed (connection error)";
            return res;
        }
        if (result->status == 404)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.read: episode_not_found (id=" +
                                std::to_string(episode_id) + ")";
            return res;
        }
        if (result->status != 200)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.read: sidecar returned HTTP " +
                                std::to_string(result->status) + ": " + result->body;
            return res;
        }

        try
        {
            res.result_json = nlohmann::json::parse(result->body);
        }
        catch (nlohmann::json::exception const& e)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = std::string("memory.read: JSON parse error: ") + e.what();
            return res;
        }

        res.outcome = DispatchResult::Outcome::Ok;
        return res;
    }
}
