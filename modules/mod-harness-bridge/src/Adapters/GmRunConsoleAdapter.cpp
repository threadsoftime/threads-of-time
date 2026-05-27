#include "Adapters/GmRunConsoleAdapter.h"

#include "ConsoleCaptureStore.h"

#include "IWorld.h"
#include "World.h"

#include <array>
#include <cstring>
#include <memory>
#include <string>
#include <string_view>

namespace HarnessBridge::Adapters
{
    // Allowlist enforced at the daemon AND here as defence in depth.
    // V1.1 expanded to .character + .tele for the smoke test.
    static constexpr std::array<char const*, 6> kAllowedPrefixes = {
        ".lookup", ".gobject", ".npc", ".bracketsets",
        ".character", ".tele",
    };

    static bool IsAllowed(std::string const& cmd)
    {
        for (auto const& prefix : kAllowedPrefixes)
        {
            if (cmd.rfind(prefix, 0) == 0)
                return true;
        }
        return false;
    }

    // Print callback. arg is a std::string* (request_id) owned by the
    // CliCommandHolder; we just read it. AC contract: Print fires zero
    // or more times BEFORE Finished, all on the worldserver main thread.
    static void CapturingPrint(void* arg, std::string_view text)
    {
        auto const* req_id = static_cast<std::string const*>(arg);
        if (req_id)
            ConsoleCaptureStore::Instance().Append(*req_id, text);
    }

    // Finished callback. AC contract: Finished fires exactly once after
    // all Print calls. We mark the capture done and delete the heap
    // string we allocated in the adapter.
    static void CapturingFinished(void* arg, bool success)
    {
        auto* req_id = static_cast<std::string*>(arg);
        if (req_id)
        {
            ConsoleCaptureStore::Instance().MarkDone(*req_id, success);
            delete req_id;
        }
    }

    DispatchResult GmRunConsole(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("command") || !args["command"].is_string())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "gm.run_console: command (string) required";
            return res;
        }
        if (!args.contains("request_id") || !args["request_id"].is_string())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "gm.run_console: request_id (string) required";
            return res;
        }
        std::string cmd = args["command"].get<std::string>();
        std::string request_id = args["request_id"].get<std::string>();
        if (!IsAllowed(cmd))
        {
            res.outcome = DispatchResult::Outcome::ValidatorRejected;
            res.error_message = "command not in allowlist (.lookup, .gobject, .npc, .bracketsets, .character, .tele)";
            return res;
        }

        // Create the capture entry BEFORE queueing the command so a
        // very fast CLI couldn't fire Finished before Insert.
        ConsoleCaptureStore::Instance().Insert(request_id);

        // Heap-allocate the request_id string, ownership transferred to
        // the CliCommandHolder. unique_ptr defends against an exception
        // before the queue insertion succeeds.
        auto arg = std::make_unique<std::string>(request_id);
        sWorld->QueueCliCommand(new CliCommandHolder(
            arg.get(), cmd.c_str(), &CapturingPrint, &CapturingFinished));
        arg.release();

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"queued",     true},
            {"request_id", request_id},
        };
        return res;
    }
}
