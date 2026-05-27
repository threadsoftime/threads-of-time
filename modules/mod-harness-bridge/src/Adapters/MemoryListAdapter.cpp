// SPDX-License-Identifier: GPL-2.0-or-later
// memory.list adapter — GET {HARNESS_MEMORY_URL}/v1/memory/{bot_guid}/episodes?...
// Per design subspec §10.5 (ListEpisodesResponse shape).
// Filters passed as query-string parameters: episode_type, entity_name,
// after, before, limit, offset.
#include "Adapters/MemoryListAdapter.h"

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

    DispatchResult MemoryList(nlohmann::json const& args)
    {
        DispatchResult res;

        if (!args.contains("bot_guid"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "memory.list: bot_guid required";
            return res;
        }

        std::string bot_guid = args["bot_guid"].get<std::string>();

        // Build query-string parameters matching the route's Query() params (§10.5).
        httplib::Params params;
        if (args.contains("episode_type") && !args["episode_type"].is_null())
            params.emplace("episode_type", args["episode_type"].get<std::string>());
        if (args.contains("entity_name") && !args["entity_name"].is_null())
            params.emplace("entity_name", args["entity_name"].get<std::string>());
        if (args.contains("after") && !args["after"].is_null())
            params.emplace("after", std::to_string(args["after"].get<int64_t>()));
        if (args.contains("before") && !args["before"].is_null())
            params.emplace("before", std::to_string(args["before"].get<int64_t>()));
        if (args.contains("limit") && !args["limit"].is_null())
            params.emplace("limit", std::to_string(args["limit"].get<int>()));
        if (args.contains("offset") && !args["offset"].is_null())
            params.emplace("offset", std::to_string(args["offset"].get<int>()));

        auto [host, port] = MemoryBaseUrl();
        httplib::Client cli(host, port);
        cli.set_connection_timeout(5);
        cli.set_read_timeout(10);

        std::string path = "/v1/memory/" + bot_guid + "/episodes";
        auto result = cli.Get(path.c_str(), params, httplib::Headers{});

        if (!result)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.list: HTTP call failed (connection error)";
            return res;
        }
        if (result->status != 200)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.list: sidecar returned HTTP " +
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
            res.error_message = std::string("memory.list: JSON parse error: ") + e.what();
            return res;
        }

        res.outcome = DispatchResult::Outcome::Ok;
        return res;
    }
}
