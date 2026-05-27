// SPDX-License-Identifier: GPL-2.0-or-later
// memory.search adapter — POST {HARNESS_MEMORY_URL}/v1/memory/{bot_guid}/search
// Per design subspec §10.4 (SearchRequest / SearchResponse shape).
// Pure nearest-neighbour search; no side effects on recall counters.
#include "Adapters/MemorySearchAdapter.h"

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

    DispatchResult MemorySearch(nlohmann::json const& args)
    {
        DispatchResult res;

        if (!args.contains("bot_guid"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "memory.search: bot_guid required";
            return res;
        }

        // Either query_text or query_vec is required (server validates; we pass through).
        if (!args.contains("query_text") && !args.contains("query_vec"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "memory.search: one of query_text or query_vec required";
            return res;
        }

        std::string bot_guid = args["bot_guid"].get<std::string>();

        // Build SearchRequest body (§10.4).
        nlohmann::json body;
        if (args.contains("query_text") && !args["query_text"].is_null())
            body["query_text"] = args["query_text"].get<std::string>();
        if (args.contains("query_vec") && args["query_vec"].is_array())
            body["query_vec"] = args["query_vec"];
        if (args.contains("top_k") && !args["top_k"].is_null())
            body["top_k"] = args["top_k"].get<int>();

        auto [host, port] = MemoryBaseUrl();
        httplib::Client cli(host, port);
        cli.set_connection_timeout(5);
        // Search may embed query_text; allow up to 15 s for the embedding round-trip.
        cli.set_read_timeout(15);

        std::string path = "/v1/memory/" + bot_guid + "/search";
        auto result = cli.Post(path.c_str(), body.dump(), "application/json");

        if (!result)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.search: HTTP call failed (connection error)";
            return res;
        }
        if (result->status != 200)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.search: sidecar returned HTTP " +
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
            res.error_message = std::string("memory.search: JSON parse error: ") + e.what();
            return res;
        }

        res.outcome = DispatchResult::Outcome::Ok;
        return res;
    }
}
