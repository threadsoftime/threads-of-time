#include "Adapters/GmTeleportAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"

namespace HarnessBridge::Adapters
{
    DispatchResult GmTeleport(nlohmann::json const& args)
    {
        DispatchResult res;
        for (auto const& key : {"target_guid", "x", "y", "z", "map"})
        {
            if (!args.contains(key))
            {
                res.outcome = DispatchResult::Outcome::BadArgs;
                res.error_message = std::string("gm.teleport: missing arg '") + key + "'";
                return res;
            }
        }
        uint64_t target = args["target_guid"].get<uint64_t>();
        float x = args["x"].get<float>();
        float y = args["y"].get<float>();
        float z = args["z"].get<float>();
        uint32 mapId = args["map"].get<uint32>();
        float o = args.value("orientation", 0.0f);

        Player* p = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(target));
        if (!p)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "player not online";
            return res;
        }

        bool ok = p->TeleportTo(mapId, x, y, z, o);
        if (!ok)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "TeleportTo returned false";
            return res;
        }
        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"ok", true}};
        return res;
    }
}
