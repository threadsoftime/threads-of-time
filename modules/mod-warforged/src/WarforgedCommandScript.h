#ifndef MOD_WARFORGED_COMMAND_SCRIPT_H
#define MOD_WARFORGED_COMMAND_SCRIPT_H

#include "Chat.h"
#include "ChatCommand.h"
#include "ObjectGuid.h"
#include "ScriptMgr.h"

namespace ModWarforged
{
    // Registers the `.warforged` GM command tree:
    //
    //   .warforged info  <itemguid>  — inspect BONUS + PRISMATIC slots on
    //                                  an item in the caller's inventory.
    //                                  Reports the enchant IDs and tags
    //                                  the BONUS slot as WARFORGED if its
    //                                  ID falls in the WF range (70001..70063).
    //   .warforged strip <itemguid>  — clear BONUS + PRISMATIC slots
    //                                  (sets both to 0, marks item changed).
    //   .warforged force <itemguid>  — directly invoke the picker + applier
    //                                  (bypasses roll-chance). Useful for QA.
    //
    // Security: SEC_GAMEMASTER. Console::No (handlers dereference
    // handler->GetSession()->GetPlayer()).
    //
    // Pattern mirrored from modules/mod-bracket-sets/src/BracketSetsCommandScript.h,
    // but using AC's typed-parameter ChatCommandBuilder (cs_debug.cpp style:
    // see HandleDebugGetItemValueCommand) instead of the legacy
    // (ChatHandler*, char const*) signature.
    class WarforgedCommandScript : public CommandScript
    {
    public:
        WarforgedCommandScript() : CommandScript("WarforgedCommandScript") {}

        Acore::ChatCommands::ChatCommandTable GetCommands() const override;

        static bool HandleWarforgedInfoCommand(ChatHandler* handler, ObjectGuid::LowType itemLowGuid);
        static bool HandleWarforgedStripCommand(ChatHandler* handler, ObjectGuid::LowType itemLowGuid);
        static bool HandleWarforgedForceCommand(ChatHandler* handler, ObjectGuid::LowType itemLowGuid);
    };
}

#endif // MOD_WARFORGED_COMMAND_SCRIPT_H
