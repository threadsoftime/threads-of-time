#include "Adapters/ObsGetInventoryAdapter.h"

#include "Bag.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "Item.h"
#include "ItemTemplate.h"

namespace HarnessBridge::Adapters
{
    static nlohmann::json ItemToJson(Item const* item, uint8 slot)
    {
        ItemTemplate const* tpl = item->GetTemplate();
        return {
            {"slot",       slot},
            {"item_entry", tpl ? tpl->ItemId : 0},
            {"name",       tpl ? tpl->Name1 : ""},
            {"itemset",    tpl ? tpl->ItemSet : 0},
            {"quality",    tpl ? tpl->Quality : 0},
            {"count",      item->GetCount()},
            {"guid",       item->GetGUID().GetRawValue()},
        };
    }

    DispatchResult ObsGetInventory(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "obs.get_inventory: target_guid (int) required";
            return res;
        }
        uint64_t target = args["target_guid"].get<uint64_t>();

        Player* p = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(target));
        if (!p)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "no player with guid " + std::to_string(target) + " online";
            return res;
        }

        nlohmann::json equipped = nlohmann::json::array();
        for (uint8 slot = EQUIPMENT_SLOT_START; slot < EQUIPMENT_SLOT_END; ++slot)
        {
            Item* it = p->GetItemByPos(INVENTORY_SLOT_BAG_0, slot);
            if (it)
                equipped.push_back(ItemToJson(it, slot));
        }

        nlohmann::json bags = nlohmann::json::array();
        for (uint8 slot = INVENTORY_SLOT_ITEM_START; slot < INVENTORY_SLOT_ITEM_END; ++slot)
        {
            Item* it = p->GetItemByPos(INVENTORY_SLOT_BAG_0, slot);
            if (it)
                bags.push_back(ItemToJson(it, slot));
        }

        // V0.3.1: equipped bag containers (slots 19-22) hold their own
        // items, invisible to the backpack-proper scan above. Reported
        // separately so existing consumers reading `bags` keep their
        // contract (slot in [23,38)) while new consumers can locate
        // items stranded by legacy strip_gear. See kb_642162c3 B4.
        nlohmann::json nested_bags = nlohmann::json::array();
        for (uint8 bag_slot = INVENTORY_SLOT_BAG_START;
             bag_slot < INVENTORY_SLOT_BAG_END;
             ++bag_slot)
        {
            Bag* container = p->GetBagByPos(bag_slot);
            if (!container)
                continue;
            nlohmann::json contents = nlohmann::json::array();
            for (uint32 sub_slot = 0; sub_slot < container->GetBagSize(); ++sub_slot)
            {
                Item* it = p->GetItemByPos(bag_slot, uint8(sub_slot));
                if (it)
                    contents.push_back(ItemToJson(it, uint8(sub_slot)));
            }
            nested_bags.push_back({
                {"bag_slot", bag_slot},
                {"capacity", container->GetBagSize()},
                {"contents", contents},
            });
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"equipped",    equipped},
            {"bags",        bags},
            {"nested_bags", nested_bags},
        };
        return res;
    }
}
