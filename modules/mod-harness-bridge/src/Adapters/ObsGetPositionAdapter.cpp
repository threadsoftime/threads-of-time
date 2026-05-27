#include "Adapters/ObsGetPositionAdapter.h"

#include "DBCStores.h"
#include "ObjectAccessor.h"
#include "Player.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetPosition(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "obs.get_position: target_guid (int) required";
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

        uint32 map_id  = p->GetMapId();
        uint32 zone_id = p->GetZoneId();
        uint32 area_id = p->GetAreaId();

        nlohmann::json out = {
            {"map_id",      map_id},
            {"zone_id",     zone_id},
            {"area_id",     area_id},
            {"x",           p->GetPositionX()},
            {"y",           p->GetPositionY()},
            {"z",           p->GetPositionZ()},
            {"orientation", p->GetOrientation()},
        };

        if (AreaTableEntry const* zone_entry = sAreaTableStore.LookupEntry(zone_id))
        {
            if (zone_entry->area_name[0] && *zone_entry->area_name[0])
                out["zone_name"] = zone_entry->area_name[0];
        }
        if (area_id != zone_id)
        {
            if (AreaTableEntry const* area_entry = sAreaTableStore.LookupEntry(area_id))
            {
                if (area_entry->area_name[0] && *area_entry->area_name[0])
                    out["area_name"] = area_entry->area_name[0];
            }
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = out;
        return res;
    }
}
