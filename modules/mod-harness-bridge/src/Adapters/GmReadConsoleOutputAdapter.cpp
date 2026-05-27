#include "Adapters/GmReadConsoleOutputAdapter.h"

#include "ConsoleCaptureStore.h"

namespace HarnessBridge::Adapters
{
    DispatchResult GmReadConsoleOutput(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("request_id") || !args["request_id"].is_string())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "gm.read_console_output: request_id (string) required";
            return res;
        }
        std::string request_id = args["request_id"].get<std::string>();
        auto cap = ConsoleCaptureStore::Instance().Read(request_id);

        res.outcome = DispatchResult::Outcome::Ok;
        if (!cap)
        {
            res.result_json = { {"found", false} };
            return res;
        }
        res.result_json = {
            {"found",     true},
            {"done",      cap->done},
            {"success",   cap->success},
            {"text",      cap->buffer},
            {"truncated", cap->truncated},
        };
        return res;
    }
}
