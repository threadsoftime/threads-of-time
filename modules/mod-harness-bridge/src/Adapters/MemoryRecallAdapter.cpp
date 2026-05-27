// SPDX-License-Identifier: GPL-2.0-or-later
// memory.recall adapter — POST {HARNESS_MEMORY_URL}/v1/memory/{bot_guid}/recall
// Per design subspec §10.3 (RecallRequest / RecallResponse shape).
// Side effect: bumps last_recalled_at + recall_count on returned episodes (§6, §10.3).
#include "Adapters/MemoryRecallAdapter.h"

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

    DispatchResult MemoryRecall(nlohmann::json const& args)
    {
        DispatchResult res;

        if (!args.contains("bot_guid") || !args.contains("query_text"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "memory.recall: bot_guid and query_text required";
            return res;
        }

        std::string bot_guid = args["bot_guid"].get<std::string>();

        // Build RecallRequest body (§10.3).
        nlohmann::json body;
        body["query_text"] = args["query_text"].get<std::string>();

        if (args.contains("top_k") && !args["top_k"].is_null())
            body["top_k"] = args["top_k"].get<int>();
        if (args.contains("entity_names") && args["entity_names"].is_array())
            body["entity_names"] = args["entity_names"];
        if (args.contains("episode_types") && args["episode_types"].is_array())
            body["episode_types"] = args["episode_types"];
        if (args.contains("time_filter") && !args["time_filter"].is_null())
            body["time_filter"] = args["time_filter"];
        if (args.contains("alpha") && !args["alpha"].is_null())
            body["alpha"] = args["alpha"].get<double>();
        if (args.contains("beta") && !args["beta"].is_null())
            body["beta"] = args["beta"].get<double>();
        if (args.contains("gamma") && !args["gamma"].is_null())
            body["gamma"] = args["gamma"].get<double>();
        if (args.contains("delta") && !args["delta"].is_null())
            body["delta"] = args["delta"].get<double>();
        if (args.contains("mmr_lambda") && !args["mmr_lambda"].is_null())
            body["mmr_lambda"] = args["mmr_lambda"].get<double>();

        auto [host, port] = MemoryBaseUrl();
        httplib::Client cli(host, port);
        cli.set_connection_timeout(5);
        // Recall involves an embedding round-trip; allow generous read timeout.
        cli.set_read_timeout(15);

        std::string path = "/v1/memory/" + bot_guid + "/recall";
        auto result = cli.Post(path.c_str(), body.dump(), "application/json");

        if (!result)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.recall: HTTP call failed (connection error)";
            return res;
        }
        if (result->status != 200)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.recall: sidecar returned HTTP " +
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
            res.error_message = std::string("memory.recall: JSON parse error: ") + e.what();
            return res;
        }

        res.outcome = DispatchResult::Outcome::Ok;
        return res;
    }
}
