// modules/mod-harness-bridge/src/Adapters/ObsGetQuestLogAdapter.cpp
#include "Adapters/ObsGetQuestLogAdapter.h"

#include "ObjectAccessor.h"
#include "ObjectMgr.h"
#include "Player.h"
#include "QuestDef.h"

namespace HarnessBridge::Adapters
{
    static char const* QuestStatusName(QuestStatus s)
    {
        switch (s)
        {
            case QUEST_STATUS_NONE:        return "none";
            case QUEST_STATUS_COMPLETE:    return "complete";
            case QUEST_STATUS_INCOMPLETE:  return "incomplete";
            case QUEST_STATUS_FAILED:      return "failed";
            case QUEST_STATUS_REWARDED:    return "rewarded";
            default:                       return "unknown";
        }
    }

    static char const* KindFor(Quest const* tpl, uint8 idx)
    {
        if (tpl->RequiredItemCount[idx] > 0)            return "item";
        if (tpl->RequiredNpcOrGo[idx] > 0)              return "kill";
        if (tpl->RequiredNpcOrGo[idx] < 0)              return "interact";
        return "none";
    }

    DispatchResult ObsGetQuestLog(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "obs.get_quest_log: target_guid (int) required";
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

        nlohmann::json quests = nlohmann::json::array();
        for (uint8 slot = 0; slot < MAX_QUEST_LOG_SIZE; ++slot)
        {
            uint32 quest_id = p->GetQuestSlotQuestId(slot);
            if (quest_id == 0)
                continue;
            Quest const* tpl = sObjectMgr->GetQuestTemplate(quest_id);
            if (!tpl)
                continue;

            nlohmann::json objectives = nlohmann::json::array();
            for (uint8 i = 0; i < QUEST_OBJECTIVES_COUNT; ++i)
            {
                uint32 cur = p->GetQuestSlotCounter(slot, i);
                int32 required = 0;
                if (tpl->RequiredItemCount[i] > 0)
                    required = tpl->RequiredItemCount[i];
                else if (tpl->RequiredNpcOrGo[i] != 0)
                    required = tpl->RequiredNpcOrGoCount[i];

                objectives.push_back({
                    {"index",    i},
                    {"current",  cur},
                    {"required", required},
                    {"kind",     KindFor(tpl, i)},
                });
            }

            quests.push_back({
                {"slot",       slot},
                {"quest_id",   quest_id},
                {"title",      tpl->GetTitle()},
                {"level",      tpl->GetQuestLevel()},
                {"status",     QuestStatusName(p->GetQuestStatus(quest_id))},
                {"objectives", objectives},
            });
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"quests", quests}};
        return res;
    }
}
