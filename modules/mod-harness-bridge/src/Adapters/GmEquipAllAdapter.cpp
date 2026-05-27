#include "Adapters/GmEquipAllAdapter.h"

#include "Bag.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "Item.h"

#include <tuple>
#include <vector>

namespace HarnessBridge::Adapters
{
    // Try to equip a single item by displacing whatever's currently in
    // the target slot back into the player's bags. Returns the
    // destination equipment slot on success, -1 if the item can't be
    // equipped at all (race/class mismatch, no eligible slot).
    //
    // `src_bag` is INVENTORY_SLOT_BAG_0 for items in the backpack proper
    // (slots 23-38) and 19..22 for items inside an equipped bag
    // container. RemoveItem must use the same (bag, slot) pair that
    // GetItemByPos used to locate the item. See kb_642162c3 B4.
    static int TryEquipWithDisplace(Player* p, Item* item, uint8 src_bag, uint8 src_slot)
    {
        // Pair-aware fast path: for finger and trinket (which have two
        // adjacent slots each), try both slots explicitly so two
        // pieces of the same pair-type land in different slots.
        // CanEquipItem(NULL_SLOT, swap=true) can otherwise resolve both
        // to slot_a and displace the first piece back.
        ItemTemplate const* proto = item->GetTemplate();
        if (proto && (proto->InventoryType == INVTYPE_FINGER
                   || proto->InventoryType == INVTYPE_TRINKET))
        {
            uint8 slot_a = (proto->InventoryType == INVTYPE_FINGER)
                           ? EQUIPMENT_SLOT_FINGER1
                           : EQUIPMENT_SLOT_TRINKET1;
            uint8 slot_b = (proto->InventoryType == INVTYPE_FINGER)
                           ? EQUIPMENT_SLOT_FINGER2
                           : EQUIPMENT_SLOT_TRINKET2;
            for (uint8 slot : { slot_a, slot_b })
            {
                if (p->GetItemByPos(INVENTORY_SLOT_BAG_0, slot))
                    continue;  // occupied
                uint16 pair_dest;
                if (p->CanEquipItem(slot, pair_dest, item, false) == EQUIP_ERR_OK)
                {
                    p->RemoveItem(src_bag, src_slot, true);
                    p->EquipItem(pair_dest, item, true);
                    return slot;
                }
            }
            // Both pair slots occupied — fall through to the V1.1 swap
            // path below.
        }

        uint16 dest;
        // swap=true so CanEquipItem allows occupied destination slots
        // (we handle the displacement ourselves below).
        InventoryResult msg = p->CanEquipItem(NULL_SLOT, dest, item, true);
        if (msg != EQUIP_ERR_OK)
            return -1;

        uint8 dst_slot = uint8(dest & 0xFF);

        // If the destination equip slot is occupied, unequip the current
        // item back to the backpack proper (slots 23-38 only — same
        // rationale as strip_gear, B4). If no backpack room, bail.
        Item* current = p->GetItemByPos(INVENTORY_SLOT_BAG_0, dst_slot);
        if (current)
        {
            bool stored = false;
            for (uint8 try_slot = INVENTORY_SLOT_ITEM_START;
                 try_slot < INVENTORY_SLOT_ITEM_END;
                 ++try_slot)
            {
                if (p->GetItemByPos(INVENTORY_SLOT_BAG_0, try_slot))
                    continue;
                ItemPosCountVec dst_pos;
                InventoryResult can_store = p->CanStoreItem(
                    INVENTORY_SLOT_BAG_0, try_slot, dst_pos, current, false);
                if (can_store == EQUIP_ERR_OK)
                {
                    p->RemoveItem(INVENTORY_SLOT_BAG_0, dst_slot, true);
                    p->StoreItem(dst_pos, current, true);
                    stored = true;
                    break;
                }
            }
            if (!stored)
                return -1;
        }

        // Now move our item from (src_bag, src_slot) to dst_slot.
        p->RemoveItem(src_bag, src_slot, true);
        p->EquipItem(dest, item, true);
        return dst_slot;
    }

    DispatchResult GmEquipAll(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "gm.equip_all: target_guid (int) required";
            return res;
        }
        uint64_t target = args["target_guid"].get<uint64_t>();
        Player* p = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(target));
        if (!p)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "player not online";
            return res;
        }

        // Collect candidate items BEFORE mutating, so we don't
        // re-encounter items that just got displaced back into the
        // same slot range. V0.3.1: scan BOTH the backpack proper AND
        // every equipped bag container (slots 19-22), so items
        // stranded in nested bags by legacy strip_gear are recovered.
        // See kb_642162c3 B4.
        std::vector<std::tuple<uint8, uint8, Item*>> candidates;  // (bag, slot, item)
        for (uint8 slot = INVENTORY_SLOT_ITEM_START; slot < INVENTORY_SLOT_ITEM_END; ++slot)
        {
            Item* item = p->GetItemByPos(INVENTORY_SLOT_BAG_0, slot);
            if (item)
                candidates.emplace_back(INVENTORY_SLOT_BAG_0, slot, item);
        }
        for (uint8 bag_slot = INVENTORY_SLOT_BAG_START;
             bag_slot < INVENTORY_SLOT_BAG_END;
             ++bag_slot)
        {
            Bag* container = p->GetBagByPos(bag_slot);
            if (!container)
                continue;
            for (uint32 sub_slot = 0; sub_slot < container->GetBagSize(); ++sub_slot)
            {
                Item* item = p->GetItemByPos(bag_slot, uint8(sub_slot));
                if (item)
                    candidates.emplace_back(bag_slot, uint8(sub_slot), item);
            }
        }

        nlohmann::json equipped = nlohmann::json::array();
        for (auto& [bag, slot, item] : candidates)
        {
            // Item may have moved since collection (prior iteration
            // displaced something into this slot). Re-fetch to be safe.
            Item* cur = p->GetItemByPos(bag, slot);
            if (!cur || cur != item)
                continue;
            int dst = TryEquipWithDisplace(p, item, bag, slot);
            if (dst >= 0)
                equipped.push_back(dst);
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"equipped_slots", equipped}};
        return res;
    }
}
