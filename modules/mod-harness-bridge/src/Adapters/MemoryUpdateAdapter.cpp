// SPDX-License-Identifier: GPL-2.0-or-later
// memory.update adapter — PATCH {HARNESS_MEMORY_URL}/v1/memory/{bot_guid}/episodes/{episode_id}
// Per design subspec §10.6 (UpdateRequest / UpdateResponse shape).
// Mutable fields: content_text (triggers re-embed), salience_score, metadata.
#include "Adapters/MemoryUpdateAdapter.h"

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

    DispatchResult MemoryUpdate(nlohmann::json const& args)
    {
        DispatchResult res;

        if (!args.contains("bot_guid") || !args.contains("episode_id"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "memory.update: bot_guid and episode_id required";
            return res;
        }

        // At least one mutable field must be present (server returns 400 otherwise,
        // but surface the error early for a cheaper code path).
        bool hasContent   = args.contains("content_text")  && !args["content_text"].is_null();
        bool hasSalience  = args.contains("salience_score") && !args["salience_score"].is_null();
        bool hasMetadata  = args.contains("metadata")       && !args["metadata"].is_null();
        if (!hasContent && !hasSalience && !hasMetadata)
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "memory.update: at least one of content_text, salience_score, metadata required";
            return res;
        }

        std::string bot_guid  = args["bot_guid"].get<std::string>();
        int64_t     episode_id = args["episode_id"].get<int64_t>();

        // Build UpdateRequest body (§10.6).
        nlohmann::json body;
        if (hasContent)
            body["content_text"]  = args["content_text"].get<std::string>();
        if (hasSalience)
            body["salience_score"] = args["salience_score"].get<double>();
        if (hasMetadata)
            body["metadata"] = args["metadata"];

        auto [host, port] = MemoryBaseUrl();
        httplib::Client cli(host, port);
        cli.set_connection_timeout(5);
        // content_text update triggers re-embedding — allow time for embedding round-trip.
        cli.set_read_timeout(15);

        std::string path = "/v1/memory/" + bot_guid + "/episodes/" + std::to_string(episode_id);
        auto result = cli.Patch(path.c_str(), body.dump(), "application/json");

        if (!result)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.update: HTTP call failed (connection error)";
            return res;
        }
        if (result->status == 404)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.update: episode_not_found (id=" +
                                std::to_string(episode_id) + ")";
            return res;
        }
        if (result->status != 200)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "memory.update: sidecar returned HTTP " +
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
            res.error_message = std::string("memory.update: JSON parse error: ") + e.what();
            return res;
        }

        res.outcome = DispatchResult::Outcome::Ok;
        return res;
    }
}
