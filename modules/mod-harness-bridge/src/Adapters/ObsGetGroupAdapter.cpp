#include "Adapters/ObsGetGroupAdapter.h"

#include "Group.h"
#include "GroupReference.h"
#include "ObjectAccessor.h"
#include "Player.h"

#include <cmath>

namespace HarnessBridge::Adapters
{
    static nlohmann::json MemberToJson(Player const* target, Player const* m)
    {
        uint32 max_hp = m->GetMaxHealth();
        double hp_pct = (max_hp > 0)
            ? std::round(static_cast<double>(m->GetHealth()) / max_hp * 1000.0) / 10.0
            : 0.0;

        nlohmann::json out = {
            {"guid",      m->GetGUID().GetCounter()},
            {"name",      m->GetName()},
            {"level",     m->GetLevel()},
            {"class_id",  m->getClass()},
            {"race_id",   m->getRace()},
            {"hp_pct",    hp_pct},
            {"in_combat", m->IsInCombat()},
            {"is_dead",   m->isDead()},
            {"distance_to_target",
             (m == target) ? 0.0 : target->GetDistance(m)},
        };

        uint32 max_mana = m->GetMaxPower(POWER_MANA);
        if (max_mana == 0)
        {
            out["mana_pct"] = nullptr;
        }
        else
        {
            double mp = std::round(
                static_cast<double>(m->GetPower(POWER_MANA)) / max_mana * 1000.0) / 10.0;
            out["mana_pct"] = mp;
        }
        return out;
    }

    DispatchResult ObsGetGroup(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "obs.get_group: target_guid (int) required";
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

        Group* g = p->GetGroup();
        if (!g)
        {
            res.outcome = DispatchResult::Outcome::Ok;
            res.result_json = {{"in_group", false}};
            return res;
        }

        nlohmann::json members = nlohmann::json::array();
        for (GroupReference* ref = g->GetFirstMember(); ref != nullptr; ref = ref->next())
        {
            Player* m = ref->GetSource();
            if (!m || !m->IsInWorld())
                continue;
            members.push_back(MemberToJson(p, m));
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"in_group",    true},
            {"group_type",  g->isRaidGroup() ? "raid" : "party"},
            {"leader_guid", g->GetLeaderGUID().GetCounter()},
            {"members",     members},
        };
        return res;
    }
}
