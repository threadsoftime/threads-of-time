#include "Adapters/GmAdditemAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"

namespace HarnessBridge::Adapters
{
    DispatchResult GmAdditem(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer() ||
            !args.contains("item_entry")  || !args["item_entry"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "gm.additem: target_guid + item_entry (ints) required";
            return res;
        }
        uint64_t target = args["target_guid"].get<uint64_t>();
        uint32_t entry  = args["item_entry"].get<uint32_t>();
        uint32_t count  = args.value("count", 1u);

        Player* p = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(target));
        if (!p)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "player not online";
            return res;
        }

        ItemPosCountVec dest;
        InventoryResult msg = p->CanStoreNewItem(NULL_BAG, NULL_SLOT, dest, entry, count);
        if (msg != EQUIP_ERR_OK)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "CanStoreNewItem failed: code " + std::to_string(msg);
            return res;
        }
        Item* it = p->StoreNewItem(dest, entry, true, Item::GenerateItemRandomPropertyId(entry));
        if (!it)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "StoreNewItem returned null";
            return res;
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"added", count}, {"item_entry", entry}, {"item_guid", it->GetGUID().GetRawValue()}};
        return res;
    }
}
