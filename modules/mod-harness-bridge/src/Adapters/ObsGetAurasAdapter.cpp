#include "Adapters/ObsGetAurasAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"
#include "SpellAuras.h"
#include "SpellAuraEffects.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetAuras(nlohmann::json const& args)
    {
        DispatchResult res;
        if (!args.contains("target_guid") || !args["target_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "obs.get_auras: target_guid (int) required";
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

        nlohmann::json auras = nlohmann::json::array();
        for (auto const& kv : p->GetAppliedAuras())
        {
            AuraApplication const* app = kv.second;
            if (!app)
                continue;
            Aura const* aura = app->GetBase();
            if (!aura)
                continue;

            nlohmann::json entry = {
                {"spell_id",        aura->GetId()},
                {"stacks",          aura->GetStackAmount()},
                {"duration_ms",     aura->GetDuration()},
                {"max_duration_ms", aura->GetMaxDuration()},
                {"source_guid",     aura->GetCasterGUID().GetRawValue()},
            };
            nlohmann::json effects = nlohmann::json::array();
            for (uint8 idx = 0; idx < MAX_SPELL_EFFECTS; ++idx)
            {
                AuraEffect* ef = aura->GetEffect(idx);
                if (!ef)
                    continue;
                effects.push_back({
                    {"index",     idx},
                    {"amount",    ef->GetAmount()},
                    {"aura_type", ef->GetAuraType()},
                });
            }
            entry["effect_amounts"] = effects;
            auras.push_back(entry);
        }

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {{"auras", auras}};
        return res;
    }
}
