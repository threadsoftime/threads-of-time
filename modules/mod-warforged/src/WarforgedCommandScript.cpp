#include "WarforgedCommandScript.h"

#include "WarforgedConstants.h"
#include "WarforgedEnchantPicker.h"
#include "WarforgedStatApplier.h"

#include "Chat.h"
#include "Item.h"
#include "ItemTemplate.h"
#include "ObjectGuid.h"
#include "Player.h"

namespace ModWarforged
{
    using namespace Acore::ChatCommands;

    ChatCommandTable WarforgedCommandScript::GetCommands() const
    {
        static ChatCommandTable warforgedCommandTable =
        {
            { "info",  HandleWarforgedInfoCommand,  SEC_GAMEMASTER, Console::No },
            { "strip", HandleWarforgedStripCommand, SEC_GAMEMASTER, Console::No },
            { "force", HandleWarforgedForceCommand, SEC_GAMEMASTER, Console::No },
        };

        static ChatCommandTable commandTable =
        {
            { "warforged", warforgedCommandTable },
        };

        return commandTable;
    }

    bool WarforgedCommandScript::HandleWarforgedInfoCommand(ChatHandler* handler, ObjectGuid::LowType itemLowGuid)
    {
        Player* p = handler->GetSession() ? handler->GetSession()->GetPlayer() : nullptr;
        if (!p)
        {
            handler->PSendSysMessage("|cffff0000.warforged info: no calling player (console not supported)|r");
            return true;
        }

        Item* item = p->GetItemByGuid(ObjectGuid::Create<HighGuid::Item>(itemLowGuid));
        if (!item)
        {
            handler->PSendSysMessage("Item %u not found in your inventory.", itemLowGuid);
            return true;
        }

        uint32 const wf  = item->GetEnchantmentId(WF_BONUS_SLOT);
        uint32 const soc = item->GetEnchantmentId(WF_SOCKET_SLOT);
        handler->PSendSysMessage("Item %u (entry %u): BONUS=%u (%s), PRISMATIC=%u",
            itemLowGuid, item->GetEntry(),
            wf, IsWarforgedEnchant(wf) ? "WARFORGED" : "non-warforged",
            soc);
        return true;
    }

    bool WarforgedCommandScript::HandleWarforgedStripCommand(ChatHandler* handler, ObjectGuid::LowType itemLowGuid)
    {
        Player* p = handler->GetSession() ? handler->GetSession()->GetPlayer() : nullptr;
        if (!p)
        {
            handler->PSendSysMessage("|cffff0000.warforged strip: no calling player (console not supported)|r");
            return true;
        }

        Item* item = p->GetItemByGuid(ObjectGuid::Create<HighGuid::Item>(itemLowGuid));
        if (!item)
        {
            handler->PSendSysMessage("Item %u not found.", itemLowGuid);
            return true;
        }

        item->SetEnchantment(WF_BONUS_SLOT,  0, 0, 0);
        item->SetEnchantment(WF_SOCKET_SLOT, 0, 0, 0);
        item->SetState(ITEM_CHANGED, p);
        handler->PSendSysMessage("Stripped Warforged + socket slots from item %u.", itemLowGuid);
        return true;
    }

    bool WarforgedCommandScript::HandleWarforgedForceCommand(ChatHandler* handler, ObjectGuid::LowType itemLowGuid)
    {
        Player* p = handler->GetSession() ? handler->GetSession()->GetPlayer() : nullptr;
        if (!p)
        {
            handler->PSendSysMessage("|cffff0000.warforged force: no calling player (console not supported)|r");
            return true;
        }

        Item* item = p->GetItemByGuid(ObjectGuid::Create<HighGuid::Item>(itemLowGuid));
        if (!item)
        {
            handler->PSendSysMessage("Item %u not found.", itemLowGuid);
            return true;
        }

        ItemTemplate const* tpl = item->GetTemplate();
        if (!tpl)
        {
            handler->PSendSysMessage("|cffff0000Item %u has no template.|r", itemLowGuid);
            return true;
        }

        uint32 const ench = ModWarforged::EnchantPicker::Pick(tpl->ItemLevel, tpl->Quality);
        bool const w = ModWarforged::StatApplier::ApplyWarforged(*item, ench);
        bool const s = ModWarforged::StatApplier::ApplySocket(*item, tpl->InventoryType);
        item->SetState(ITEM_CHANGED, p);
        handler->PSendSysMessage(
            "Forced rolls on item %u (entry %u, ilvl %u, q %u): picked ench=%u, warforged=%d socket=%d",
            itemLowGuid, item->GetEntry(),
            tpl->ItemLevel, uint32(tpl->Quality),
            ench, int(w), int(s));
        return true;
    }
}
