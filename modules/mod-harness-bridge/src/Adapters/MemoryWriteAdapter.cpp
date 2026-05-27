// SPDX-License-Identifier: GPL-2.0-or-later
// memory.write adapter — POST {HARNESS_MEMORY_URL}/v1/memory/{bot_guid}/episodes
// Per design subspec §10.1 (MemoryWriteArgs request / WriteEpisodeResponse shape).
#include "Adapters/MemoryWriteAdapter.h"

#include "Bot/LlmAgent/Vendor/httplib.h"
#include "Log.h"

#include <cstdlib>
#include <string>

namespace HarnessBridge::Adapters
{
    namespace
    {
        // Parse "http://host:port" into {host, port}.  Returns {"localhost", 8090}
        // for any malformed / missing value so behaviour is predictable in all
        // environments.
        std::pair<std::string, int> MemoryBaseUrl()
        {
            const char* env = std::getenv("HARNESS_MEMORY_URL");
            std::string url = env ? env : "http://localhost:8090";

            // Strip scheme prefix.
            auto schemeEnd = url.find("://");
            std::string authority = (schemeEnd == std::string::npos)
                ? url
                : url.substr(schemeEnd + 3);

            // Strip any trailing path.
            auto slashPos = authority.find('/');
            if (slashPos != std::string::npos)
                authority = authority.substr(0, slashPos);

            // Split host:port.
            auto colonPos = authority.rfind(':');
            if (colonPos == std::string::npos)
                return {authority, 8090};

            std::string host = authority.substr(0, colonPos);
            int port = std::stoi(authority.substr(colonPos + 1));
            return {host, port};
        }
    }  // namespace

    DispatchResult MemoryWrite(nlohmann::json const& args)
    {
        DispatchResult res;

        if (!args.contains("bot_guid") || !args.contains("content_text") ||
            !args.contains("episode_type") || !args.contains("timestamp"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "memory.write: bot_guid, content_text, episode_type, timestamp required";
            return res;
        }

        std::string bot_guid = args["bot_guid"].get<std::string>();

        // Build the request body matching WriteEpisodeRequest (§10.1).
        nlohmann::json body;
        body["content_text"]  = args["content_text"].get<std::string>();
        body["episode_type"]  = args["episode_type"].get<std::string>();
        body["timestamp"]     = args["timestamp"].get<int64_t>();

        if (args.contains("salience_hint") && !args["salience_hint"].is_null())
            body["salience_hint"] = args["salience_hint"].get<double>();
        if (args.contains("entities") && args["entities"].is_array())
            body["entities"] = args["entities"];
        if (args.contains("metadata") && !args["metadata"].is_null())
            body["metadata"] = args["metadata"];
        if (args.contains("source") && !args["source"].is_null())
            body["source"] = args["source"].get<std::string>();

        auto [host, port] = MemoryBaseUrl();
        httplib::Client cli(host, port);
        cli.set_connection_timeout(5);
        cli.set_read_timeout(10);

        std::string path = "/v1/memory/" + bot_guid + "/episodes";
        auto result = cli.Post(path.c_str(), body.dump(), "application/json");

        if (!result)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.write: HTTP call failed (connection error)";
            return res;
        }
        if (result->status != 201)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.write: sidecar returned HTTP " +
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
            res.error_message = std::string("memory.write: JSON parse error: ") + e.what();
            return res;
        }

        res.outcome = DispatchResult::Outcome::Ok;
        return res;
    }
}
