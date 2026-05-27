#include "Adapters/ObsGetTalentsAdapter.h"

#include "DBCStores.h"
#include "DBCStructure.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "SpellInfo.h"
#include "SpellMgr.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetTalents(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "obs.get_talents: target_guid (int) required";
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

        uint8 active_spec = p->GetActiveSpec();
        uint8 spec_mask   = p->GetActiveSpecMask();

        nlohmann::json talents = nlohmann::json::array();

        // Iterate the player's talent map (mirror AiFactory::GetPlayerSpecTabs:103-132).
        PlayerTalentMap const& talentMap = p->GetTalentMap();
        for (auto const& [spell_id, talent] : talentMap)
        {
            if (!talent) continue;
            if ((spec_mask & talent->specMask) == 0) continue;

            TalentSpellPos const* pos = GetTalentSpellPos(spell_id);
            if (!pos) continue;

            TalentEntry const* info = sTalentStore.LookupEntry(pos->talent_id);
            if (!info) continue;

            SpellInfo const* spell = sSpellMgr->GetSpellInfo(spell_id);
            int rank = spell ? spell->GetRank() : (pos->rank + 1);

            // PlayerTalent::specMask is `uint8 specMask:8` (a bit-field) per
            // Player.h:138 — nlohmann::json cannot bind a bit-field by
            // reference, so cast through a local before emitting.
            uint32 spec_mask_val = talent->specMask;
            talents.push_back({
                {"spell_id",  spell_id},
                {"talent_id", pos->talent_id},
                {"tab",       info->TalentTab},
                {"row",       info->Row},
                {"col",       info->Col},
                {"rank",      rank},
                {"spec_mask", spec_mask_val},
            });
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"active_spec", active_spec},
            {"specs_count", p->GetSpecsCount()},
            {"talents",     talents},
        };
        return res;
    }
}
