#include "BracketSetsClientItemHook.h"
#include "BracketSetsConstants.h"
#include "BracketSetsManager.h"
#include "Item.h"
#include "ItemTemplate.h"
#include "Log.h"
#include "Opcodes.h"
#include "Player.h"
#include "WorldPacket.h"
#include "WorldSession.h"
#include <set>

using BracketSets::SENTINEL_ITEMSET_ID;
using BracketSets::BRACKET_1;
using BracketSets::DetermineSpec;

BracketSetsClientItemHook::BracketSetsClientItemHook()
    : PlayerScript("BracketSetsClientItemHook", {
        PLAYERHOOK_ON_BUILD_ITEM_QUERY_RESPONSE,
        PLAYERHOOK_ON_TALENTS_RESET,
        PLAYERHOOK_ON_AFTER_SPEC_SLOT_CHANGED
    })
{}

void BracketSetsClientItemHook::OnPlayerBuildItemQueryResponse(Player* player, ItemTemplate const* proto, uint32& itemSet)
{
    if (!player || !proto) return;
    if (itemSet != SENTINEL_ITEMSET_ID) return;  // not a Bracket 1 piece

    uint8 classId = player->getClass();
    uint8 specId  = DetermineSpec(player);  // talent-tree dominant index (1/2/3), matches server-side aura logic

    uint32 mappedItemset = BracketSets::ResolveItemset(classId, specId, BRACKET_1);
    if (mappedItemset == 0) return;  // edge case §7.2 — DK/unmapped; leave 90101, client renders fallback row

    itemSet = mappedItemset;

    LOG_INFO("module",
        "[mod-bracket-sets] item-query rewrite: guid={} class={} spec={} item={} 90101->{}",
        player->GetGUID().GetCounter(), classId, specId, proto->ItemId, mappedItemset);
}

namespace {

// Push a fresh SMSG_ITEM_QUERY_SINGLE_RESPONSE for every Bracket 1 piece in
// the player's equipped slots. Mirrors the iteration pattern used by
// BracketSetsManager::CountEquippedSetPieces (equipped-only; bags and bank are
// intentionally excluded for V1 — items pulled from bags trigger a natural
// CMSG_ITEM_QUERY_SINGLE that flows through hook 3a anyway).
//
// De-dupes by item entry so that N identical pieces only cause one push.
// The synthetic handler call flows through OnPlayerBuildItemQueryResponse,
// rewriting itemset with the player's CURRENT spec and overwriting the
// client's stale cache entry. See spec §8.1.
void RefreshBracket1Items(Player* player)
{
    if (!player || !player->GetSession()) return;

    std::set<uint32> entries;

    for (uint8 slot = EQUIPMENT_SLOT_START; slot < EQUIPMENT_SLOT_END; ++slot)
    {
        Item* item = player->GetItemByPos(INVENTORY_SLOT_BAG_0, slot);
        if (!item)
            continue;
        ItemTemplate const* proto = item->GetTemplate();
        if (proto && proto->ItemSet == SENTINEL_ITEMSET_ID)
            entries.insert(item->GetEntry());
    }

    if (entries.empty())
        return;

    // Forge a CMSG_ITEM_QUERY_SINGLE for each unique entry and call the handler
    // synthetically. The core handler builds + sends SMSG_ITEM_QUERY_SINGLE_RESPONSE,
    // which flows through OnPlayerBuildItemQueryResponse above, rewriting itemset
    // for the player's current spec. The client overwrites its cache on receipt.
    for (uint32 entry : entries)
    {
        WorldPacket fakeRecv(CMSG_ITEM_QUERY_SINGLE, 4);
        fakeRecv << entry;
        player->GetSession()->HandleItemQuerySingleOpcode(fakeRecv);
    }

    LOG_INFO("module", "[mod-bracket-sets] respec refresh: pushed {} item-query response(s) for player guid={}",
             entries.size(), player->GetGUID().GetCounter());
}

}  // anonymous namespace

void BracketSetsClientItemHook::OnPlayerTalentsReset(Player* player, bool /*noCost*/)
{
    RefreshBracket1Items(player);
}

void BracketSetsClientItemHook::OnPlayerAfterSpecSlotChanged(Player* player, uint8 /*newSlot*/)
{
    RefreshBracket1Items(player);
}
