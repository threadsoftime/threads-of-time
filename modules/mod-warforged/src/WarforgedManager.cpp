#include "WarforgedManager.h"
#include "WarforgedConfig.h"
#include "WarforgedConstants.h"
#include "WarforgedRng.h"
#include "WarforgedEnchantPicker.h"
#include "WarforgedStatApplier.h"
#include "Item.h"
#include "ItemTemplate.h"
#include "Player.h"
#include "Log.h"

namespace ModWarforged::Manager
{
    bool IsEligible(Item* item)
    {
        if (!item) return false;
        ItemTemplate const* tpl = item->GetTemplate();
        if (!tpl) return false;

        // Quality gate
        if (tpl->Quality < gConfig.minQuality || tpl->Quality > gConfig.maxQuality)
            return false;

        // Skip quest items
        if (tpl->Class == ITEM_CLASS_QUEST) return false;

        // Skip stackable consumables (food, potions, etc.)
        if (tpl->Stackable > 1) return false;

        return true;
    }

    RollResult RollAndApply(Player* player, Item* item)
    {
        RollResult r;
        if (!gConfig.enable) return r;
        if (!IsEligible(item)) return r;

        bool rollWf  = Rng::RollWarforged();
        bool rollSoc = Rng::RollSocket();

        if (rollWf)
        {
            ItemTemplate const* tpl = item->GetTemplate();
            uint32 enchantId = EnchantPicker::Pick(tpl->ItemLevel, tpl->Quality);
            if (enchantId != 0)
                r.warforged = StatApplier::ApplyWarforged(*item, enchantId);
        }

        // Socket gate — the ToT bracket design locks players in pre-Outland
        // phases through L60, but Jewelcrafting + AH-sourced gems become realistic
        // around L56. Below that, a procced socket is dead inventory.
        if (rollSoc && player->GetLevel() >= gConfig.socketMinCharLevel)
        {
            uint8 invtype = item->GetTemplate()->InventoryType;
            r.socket = StatApplier::ApplySocket(*item, invtype);
        }

        if (r.warforged || r.socket)
        {
            item->SetState(ITEM_CHANGED, player);
            LOG_DEBUG("server.loading",
                      "[mod-warforged] RollAndApply: player {} item entry {} → warforged={} socket={}",
                      player->GetName(), item->GetEntry(), r.warforged, r.socket);
        }

        return r;
    }
}
