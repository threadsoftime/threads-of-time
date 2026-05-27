#include "Adapters/BotSendChatAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"
#include "Script/Playerbots.h"

#include <string>

namespace HarnessBridge::Adapters
{
    DispatchResult BotSendChat(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("bot_guid") || !args.contains("channel") || !args.contains("message"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.send_chat: bot_guid + channel + message required";
            return res;
        }
        uint64_t target  = args["bot_guid"].get<uint64_t>();
        std::string chan = args["channel"].get<std::string>();
        std::string msg  = args["message"].get<std::string>();
        std::string recipient_name = args.value("recipient_name", std::string(""));

        // Validate channel + per-channel preconditions before player lookup.
        bool valid_channel =
            chan == "say" || chan == "yell" || chan == "party" ||
            chan == "raid" || chan == "guild" || chan == "whisper";
        if (!valid_channel)
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message =
                "bot.send_chat: channel must be one of "
                "\"say\", \"yell\", \"party\", \"raid\", \"guild\", \"whisper\"; got \"" + chan + "\"";
            return res;
        }
        if (chan == "whisper" && recipient_name.empty())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.send_chat: recipient_name required when channel=\"whisper\"";
            return res;
        }
        if (msg.empty())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "bot.send_chat: message must be non-empty";
            return res;
        }

        Player* p = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(target));
        if (!p)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "bot not online";
            return res;
        }
        PlayerbotAI* ai = GET_PLAYERBOT_AI(p);
        if (!ai)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "player is not a bot";
            return res;
        }

        bool sent = false;
        if      (chan == "say")     sent = ai->Say(msg);
        else if (chan == "yell")    sent = ai->Yell(msg);
        else if (chan == "party")   sent = ai->SayToParty(msg);
        else if (chan == "raid")    sent = ai->SayToRaid(msg);
        else if (chan == "guild")   sent = ai->SayToGuild(msg);
        else if (chan == "whisper") sent = ai->Whisper(msg, recipient_name);

        if (!sent)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message =
                "bot.send_chat: underlying " + chan + " returned false "
                "(common causes: no group/raid/guild, recipient offline, no rights)";
            return res;
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"sent",           true},
            {"channel",        chan},
            {"message_length", static_cast<int>(msg.size())},
        };
        return res;
    }
}
