#include "Handlers/DispatchHandler.h"
#include "HarnessBridgeConfig.h"
#include "HarnessBridgeDispatch.h"

#include "Bot/LlmAgent/Vendor/httplib.h"
#include "Bot/LlmAgent/Vendor/nlohmann_json.hpp"

#include <chrono>
#include <future>

namespace HarnessBridge
{
    namespace
    {
        nlohmann::json BuildErrorBody(std::string const& code, std::string const& msg)
        {
            return nlohmann::json{
                {"ok",    false},
                {"error", code},
                {"detail", msg},
            };
        }

        void WriteError(httplib::Response& res, int status, std::string const& code, std::string const& msg)
        {
            res.status = status;
            res.set_content(BuildErrorBody(code, msg).dump(), "application/json");
        }
    }

    void RegisterDispatchHandler(httplib::Server& server)
    {
        server.Post("/dispatch", [](httplib::Request const& req, httplib::Response& res) {
            nlohmann::json body;
            try
            {
                body = nlohmann::json::parse(req.body);
            }
            catch (std::exception const& e)
            {
                WriteError(res, 400, "bad_request", std::string("json parse: ") + e.what());
                return;
            }

            if (!body.contains("tool") || !body["tool"].is_string())
            {
                WriteError(res, 400, "bad_request", "missing 'tool' string");
                return;
            }

            auto work = std::make_unique<PendingWork>();
            work->tool_name   = body["tool"].get<std::string>();
            work->args_json   = body.contains("args") ? body["args"] : nlohmann::json::object();
            work->request_id  = req.get_header_value("X-Daemon-Request-Id");
            work->identity    = req.get_header_value("X-Daemon-Identity");
            work->enqueued_at = std::chrono::steady_clock::now();

            auto future = work->result_promise.get_future();

            if (!Queue().TryPush(std::move(work)))
            {
                res.status = 503;
                res.set_header("Retry-After", "1");
                res.set_content(BuildErrorBody("unavailable", "queue full").dump(), "application/json");
                return;
            }

            auto timeout_ms = GetConfig().RequestTimeoutMs;
            auto status = future.wait_for(std::chrono::milliseconds(timeout_ms));
            if (status != std::future_status::ready)
            {
                WriteError(res, 504, "timeout",
                           "main-thread drain did not respond within " + std::to_string(timeout_ms) + " ms");
                return;
            }

            DispatchResult result = future.get();

            nlohmann::json out = {
                {"ok",            result.outcome == DispatchResult::Outcome::Ok},
                {"result",        result.result_json},
                {"tick_wait_ms",  result.tick_wait_ms},
                {"executor_ms",   result.executor_ms},
            };

            switch (result.outcome)
            {
                case DispatchResult::Outcome::Ok:
                    res.status = 200;
                    break;
                case DispatchResult::Outcome::UnknownTool:
                    res.status = 404;
                    out["error"] = "unknown_tool";
                    out["detail"] = result.error_message;
                    break;
                case DispatchResult::Outcome::BadArgs:
                    res.status = 400;
                    out["error"] = "bad_request";
                    out["detail"] = result.error_message;
                    break;
                case DispatchResult::Outcome::ValidatorRejected:
                    res.status = 409;
                    out["error"] = "validator_rejected";
                    out["detail"] = result.error_message;
                    break;
                case DispatchResult::Outcome::ExecutorFailed:
                case DispatchResult::Outcome::ExecutorThrew:
                    res.status = 422;
                    out["error"] = (result.outcome == DispatchResult::Outcome::ExecutorThrew) ? "executor_threw" : "executor_failed";
                    out["detail"] = result.error_message;
                    break;
                default:
                    res.status = 500;
                    out["error"] = "internal";
                    out["detail"] = result.error_message;
                    break;
            }

            res.set_content(out.dump(), "application/json");
        });
    }
}
