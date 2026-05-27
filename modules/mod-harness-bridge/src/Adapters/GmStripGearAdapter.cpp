#include "Adapters/GmStripGearAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"

#include <vector>

namespace HarnessBridge::Adapters
{
    DispatchResult GmStripGear(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "gm.strip_gear: target_guid (int) required";
            return res;
        }
        uint64_t target = args["target_guid"].get<uint64_t>();
        bool destroy_if_full = args.value("destroy_if_full", false);
        bool clear_bag = args.value("clear_bag", false);

        Player* p = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(target));
        if (!p)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "no player with guid " + std::to_string(target) + " online";
            return res;
        }

        std::vector<uint8> stripped;
        std::vector<uint8> destroyed;
        std::vector<uint8> cleared_bag;

        for (uint8 slot = EQUIPMENT_SLOT_START; slot < EQUIPMENT_SLOT_END; ++slot)
        {
            Item* item = p->GetItemByPos(INVENTORY_SLOT_BAG_0, slot);
            if (!item)
                continue;

            // V0.3.1: place stripped items in BACKPACK PROPER only
            // (INVENTORY_SLOT_BAG_0 slots 23-38). The legacy
            // CanStoreItem(NULL_BAG, NULL_SLOT, ...) overflowed into
            // equipped bag containers, where the V1.x equip_all and
            // obs.get_inventory don't iterate — items appeared lost.
            // See kb_642162c3 B4.
            bool placed = false;
            for (uint8 dst_slot = INVENTORY_SLOT_ITEM_START;
                 dst_slot < INVENTORY_SLOT_ITEM_END;
                 ++dst_slot)
            {
                if (p->GetItemByPos(INVENTORY_SLOT_BAG_0, dst_slot))
                    continue;  // already occupied
                ItemPosCountVec dst;
                InventoryResult can = p->CanStoreItem(
                    INVENTORY_SLOT_BAG_0, dst_slot, dst, item, false);
                if (can == EQUIP_ERR_OK)
                {
                    p->RemoveItem(INVENTORY_SLOT_BAG_0, slot, true);
                    p->StoreItem(dst, item, true);
                    stripped.push_back(slot);
                    placed = true;
                    break;
                }
            }
            if (placed)
                continue;

            if (destroy_if_full)
            {
                p->DestroyItem(INVENTORY_SLOT_BAG_0, slot, true);
                destroyed.push_back(slot);
            }
            else
            {
                res.outcome = DispatchResult::Outcome::ValidatorRejected;
                res.error_message = "backpack full at equipment slot " + std::to_string(slot)
                                  + "; pass destroy_if_full=true to force";
                res.result_json = {
                    {"stripped_slots", stripped},
                    {"destroyed_slots", destroyed},
                };
                return res;
            }
        }

        // Optional second pass: destroy backpack contents (slots 23-38).
        // The smoke suite needs this when reshaping bots — otherwise the
        // bot's stripped gear sits in the bag and gets re-equipped by
        // the subsequent gm.equip_all call, displacing the bracket-1
        // pieces we just added.
        if (clear_bag)
        {
            for (uint8 slot = INVENTORY_SLOT_ITEM_START; slot < INVENTORY_SLOT_ITEM_END; ++slot)
            {
                Item* item = p->GetItemByPos(INVENTORY_SLOT_BAG_0, slot);
                if (!item)
                    continue;
                p->DestroyItem(INVENTORY_SLOT_BAG_0, slot, true);
                cleared_bag.push_back(slot);
            }
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"stripped_slots", stripped},
            {"destroyed_slots", destroyed},
            {"cleared_bag_slots", cleared_bag},
        };
        return res;
    }
}
