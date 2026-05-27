// SPDX-License-Identifier: GPL-2.0-or-later
// memory.delete adapter — DELETE {HARNESS_MEMORY_URL}/v1/memory/{bot_guid}/episodes/{episode_id}
// Per design subspec §10.7. Hard-delete: cascades to episode_entities via FK;
// embeddings_vec row deleted explicitly by the server route (no FK cascade on vec0).
// Server returns 204 No Content on success; 404 when episode absent.
#include "Adapters/MemoryDeleteAdapter.h"

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

    DispatchResult MemoryDelete(nlohmann::json const& args)
    {
        DispatchResult res;

        if (!args.contains("bot_guid") || !args.contains("episode_id"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "memory.delete: bot_guid and episode_id required";
            return res;
        }

        std::string bot_guid   = args["bot_guid"].get<std::string>();
        int64_t     episode_id = args["episode_id"].get<int64_t>();

        auto [host, port] = MemoryBaseUrl();
        httplib::Client cli(host, port);
        cli.set_connection_timeout(5);
        cli.set_read_timeout(10);

        std::string path = "/v1/memory/" + bot_guid + "/episodes/" + std::to_string(episode_id);
        auto result = cli.Delete(path.c_str());

        if (!result)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.delete: HTTP call failed (connection error)";
            return res;
        }
        if (result->status == 404)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.delete: episode_not_found (id=" +
                                std::to_string(episode_id) + ")";
            return res;
        }
        if (result->status != 204)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.delete: sidecar returned HTTP " +
                                std::to_string(result->status) + ": " + result->body;
            return res;
        }

        // 204 No Content — success, no response body.
        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"deleted", true}, {"episode_id", episode_id}};
        return res;
    }
}
