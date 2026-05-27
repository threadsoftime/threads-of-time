#include "HeimbotCommandScript.h"

#include "Chat.h"
#include "HeimbotSettings.h"
#include "Player.h"
#include "RotationModeConfig.h"
#include "ScriptMgr.h"

namespace
{
    using namespace Acore::ChatCommands;

    static bool HandleMode(ChatHandler* handler, char const* args)
    {
        if (!RotationMode::GetConfig().Enabled)
        {
            handler->PSendSysMessage("|cffff0000mod-rotation-mode is disabled in server config|r");
            return true;
        }

        Player* player = handler->GetSession() ? handler->GetSession()->GetPlayer() : nullptr;
        if (!player)
            return false;

        std::string arg = args ? args : "";
        if (arg.empty())
        {
            handler->PSendSysMessage("usage: .heimbot mode <off|rotation|grind|squad-mirror>");
            handler->PSendSysMessage("current: {}", RotationMode::ModeToString(RotationMode::GetMode(player->GetGUID().GetCounter())));
            return true;
        }

        auto mode = RotationMode::ModeFromString(arg);
        RotationMode::SetMode(player->GetGUID().GetCounter(), mode);
        handler->PSendSysMessage("heimbot mode -> {}", RotationMode::ModeToString(mode));

        // v0 scaffold: persisting only. Step 3+ will apply the corresponding
        // strategy set to the player's PlayerbotAI (toggle self-bot, ON/OFF
        // strategy lists per class+spec, optional HealTargetSourceStrategy
        // override).

        return true;
    }

    static bool HandleSet(ChatHandler* handler, char const* args)
    {
        if (!args || !*args)
        {
            handler->PSendSysMessage("usage: .heimbot set <key> <value>");
            return true;
        }

        Player* player = handler->GetSession() ? handler->GetSession()->GetPlayer() : nullptr;
        if (!player)
            return false;

        std::string s = args;
        size_t sp = s.find(' ');
        if (sp == std::string::npos)
        {
            handler->PSendSysMessage("usage: .heimbot set <key> <value>");
            return true;
        }
        std::string key = s.substr(0, sp);
        std::string val = s.substr(sp + 1);

        RotationMode::Set(player->GetGUID().GetCounter(), key, val);
        handler->PSendSysMessage("heimbot {} = {}", key, val);
        return true;
    }

    static bool HandleGet(ChatHandler* handler, char const* args)
    {
        if (!args || !*args)
        {
            handler->PSendSysMessage("usage: .heimbot get <key>");
            return true;
        }

        Player* player = handler->GetSession() ? handler->GetSession()->GetPlayer() : nullptr;
        if (!player)
            return false;

        auto val = RotationMode::Get(player->GetGUID().GetCounter(), args);
        if (val)
            handler->PSendSysMessage("heimbot {} = {}", args, *val);
        else
            handler->PSendSysMessage("heimbot {} = (not set)", args);
        return true;
    }

    static bool HandleList(ChatHandler* handler, char const* /*args*/)
    {
        Player* player = handler->GetSession() ? handler->GetSession()->GetPlayer() : nullptr;
        if (!player)
            return false;

        handler->PSendSysMessage("heimbot settings (character {}):", player->GetGUID().GetCounter());
        handler->PSendSysMessage("  mode = {}", RotationMode::ModeToString(RotationMode::GetMode(player->GetGUID().GetCounter())));
        // v0 scaffold: only the mode field is surfaced. Step 2 of PE2
        // implements a full SELECT + iteration.
        return true;
    }

    static bool HandleReset(ChatHandler* handler, char const* /*args*/)
    {
        Player* player = handler->GetSession() ? handler->GetSession()->GetPlayer() : nullptr;
        if (!player)
            return false;

        RotationMode::ResetAll(player->GetGUID().GetCounter());
        handler->PSendSysMessage("heimbot settings reset to defaults");
        return true;
    }

    class HeimbotCommandScript : public CommandScript
    {
    public:
        HeimbotCommandScript() : CommandScript("HeimbotCommandScript") {}

        ChatCommandTable GetCommands() const override
        {
            static ChatCommandTable heimbotCommandTable =
            {
                { "mode",  HandleMode,  SEC_PLAYER, Console::No },
                { "set",   HandleSet,   SEC_PLAYER, Console::No },
                { "get",   HandleGet,   SEC_PLAYER, Console::No },
                { "list",  HandleList,  SEC_PLAYER, Console::No },
                { "reset", HandleReset, SEC_PLAYER, Console::No },
            };

            static ChatCommandTable commandTable =
            {
                { "heimbot", heimbotCommandTable },
            };

            return commandTable;
        }
    };
}

namespace RotationMode
{
    void RegisterCommandScript()
    {
        new ::HeimbotCommandScript();
    }
}
