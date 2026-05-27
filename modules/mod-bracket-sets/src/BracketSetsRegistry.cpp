#include "BracketSetsRegistry.h"

#include "DatabaseEnv.h"
#include "Field.h"
#include "Log.h"
#include "QueryResult.h"

#include <set>
#include <utility>

namespace BracketSets
{
    namespace
    {
        BonusMap gBonuses;
    }

    bool IsBracketSet(uint32 itemset_id)
    {
        return itemset_id >= 90101 && itemset_id <= 90199;
    }

    BonusMap const& Bonuses()
    {
        return gBonuses;
    }

    size_t BonusCount()
    {
        return gBonuses.size();
    }

    BonusRow const* Lookup(BonusKey const& key)
    {
        auto it = gBonuses.find(key);
        return it == gBonuses.end() ? nullptr : &it->second;
    }

    std::string LookupDisplayNameBySpell(uint32 spell_id)
    {
        for (auto const& kv : gBonuses)
            if (kv.second.spell_id == spell_id)
                return kv.second.display_name;
        return {};
    }

    void LoadBonusMap()
    {
        gBonuses.clear();

        QueryResult result = WorldDatabase.Query(
            "SELECT `itemset_id`, `threshold`, `class_id`, `spec_id`, "
            "`spell_id`, `bracket_min`, `bracket_max`, `display_name` "
            "FROM `bracket_set_bonus_map`"
        );
        if (!result)
        {
            LOG_WARN("server.loading",
                     "[mod-bracket-sets] bracket_set_bonus_map is empty or missing — bonuses disabled");
            return;
        }

        do
        {
            Field* fields = result->Fetch();
            BonusKey key {
                fields[0].Get<uint32>(),
                fields[1].Get<uint8>(),
                fields[2].Get<uint8>(),
                fields[3].Get<uint8>(),
            };
            BonusRow row {
                fields[4].Get<uint32>(),
                fields[5].Get<uint8>(),
                fields[6].Get<uint8>(),
                fields[7].Get<std::string>(),
            };
            gBonuses.emplace(key, std::move(row));
        } while (result->NextRow());

        // Tally distinct itemsets for the log line.
        std::set<uint32> itemsets;
        for (auto const& kv : gBonuses)
            itemsets.insert(kv.first.itemset_id);

        LOG_INFO("server.loading",
                 "[mod-bracket-sets] loaded N={} bonus mappings for {} itemset(s)",
                 gBonuses.size(), itemsets.size());
    }

    void ReloadBonusMap()
    {
        LoadBonusMap();
    }
}
