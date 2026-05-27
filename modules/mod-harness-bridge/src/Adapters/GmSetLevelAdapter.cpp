#include "Adapters/GmSetLevelAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"

namespace HarnessBridge::Adapters
{
    DispatchResult GmSetLevel(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args.contains("level"))
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "gm.set_level: target_guid + level (ints) required";
            return res;
        }
        uint64_t target = args["target_guid"].get<uint64_t>();
        uint8 level = static_cast<uint8>(args["level"].get<int>());

        Player* p = ObjectAccessor::FindPlayer(ObjectGuid::Create<HighGuid::Player>(target));
        if (!p)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "player not online";
            return res;
        }
        uint8 prev = p->GetLevel();
        p->GiveLevel(level);
        p->InitTalentForLevel();
        p->SetUInt32Value(PLAYER_XP, 0);

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"previous", prev}, {"new", level}};
        return res;
    }
}
