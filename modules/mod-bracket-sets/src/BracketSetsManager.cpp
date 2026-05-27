#include "BracketSetsManager.h"

#include "BracketSetsConfig.h"
#include "BracketSetsRegistry.h"
#include "DatabaseEnv.h"
#include "Field.h"
#include "Item.h"
#include "ItemTemplate.h"
#include "Log.h"
#include "Player.h"
#include "QueryResult.h"

#include <unordered_map>
#include <unordered_set>

namespace BracketSets
{
    namespace
    {
        ItemsetResolver gItemsetResolver;
    }

    void LoadItemsetMap()
    {
        gItemsetResolver.Clear();

        QueryResult result = WorldDatabase.Query(
            "SELECT `class_id`, `spec_id`, `bracket_id`, `itemset_id` "
            "FROM `bracket_set_itemset_map`"
        );
        if (!result)
        {
            LOG_WARN("server.loading",
                     "[mod-bracket-sets] bracket_set_itemset_map is empty or missing — itemset resolver disabled");
            return;
        }

        uint32 count = 0;
        do
        {
            Field* fields = result->Fetch();
            gItemsetResolver.Insert(
                fields[0].Get<uint8>(),
                fields[1].Get<uint8>(),
                fields[2].Get<uint8>(),
                fields[3].Get<uint32>()
            );
            ++count;
        } while (result->NextRow());

        LOG_INFO("server.loading",
                 "[mod-bracket-sets] loaded N={} itemset mappings",
                 count);
    }

    uint32 ResolveItemset(uint8 classId, uint8 specId, uint8 bracketId)
    {
        return static_cast<uint32>(gItemsetResolver.Resolve(classId, specId, bracketId));
    }

    uint8 DetermineSpec(Player* player)
    {
        uint8 points[3] = {0, 0, 0};
        player->GetTalentTreePoints(points);
        uint8 best = 0;
        for (uint8 i = 1; i < 3; ++i)
            if (points[i] > points[best])
                best = i;
        return best + 1;
    }

    namespace
    {
        // Walk the player's equipped armor slots and count items per managed
        // itemset (90101..90199). Weapons, shields, and off-set items are
        // skipped (their item_template.itemset is 0).
        std::unordered_map<uint32, uint8> CountEquippedSetPieces(Player* player)
        {
            std::unordered_map<uint32, uint8> counts;
            for (uint8 slot = EQUIPMENT_SLOT_START; slot < EQUIPMENT_SLOT_END; ++slot)
            {
                Item* item = player->GetItemByPos(INVENTORY_SLOT_BAG_0, slot);
                if (!item)
                    continue;
                ItemTemplate const* proto = item->GetTemplate();
                if (!proto)
                    continue;
                uint32 setid = proto->ItemSet;
                if (!IsBracketSet(setid))
                    continue;
                ++counts[setid];
            }
            return counts;
        }
    }

    void ReevalAll(Player* player)
    {
        if (!player || !GetConfig().Enabled)
            return;

        uint8 const class_id = player->getClass();
        uint8 const spec_id  = DetermineSpec(player);
        uint8 const level    = player->GetLevel();

        auto counts = CountEquippedSetPieces(player);

        // Build the set of spell IDs that should be applied right now.
        std::unordered_set<uint32> desired;
        for (auto const& kv : counts)
        {
            uint32 itemset_id = kv.first;
            uint8 count       = kv.second;
            for (uint8 threshold : {uint8(2), uint8(4)})
            {
                if (count < threshold)
                    continue;
                BonusKey key{itemset_id, threshold, class_id, spec_id};
                BonusRow const* row = Lookup(key);
                if (!row)
                    continue;
                if (level < row->bracket_min || level > row->bracket_max)
                    continue;
                desired.insert(row->spell_id);
            }
        }

        // Reconcile: apply missing desired, remove orphaned managed auras.
        // We iterate the full registry rather than tracking per-player state —
        // each marker spell ID is uniquely ours, so HasAura(id) is a sufficient
        // proxy for "this is one of ours currently on the player."
        size_t applied = 0;
        size_t removed = 0;
        for (auto const& kv : Bonuses())
        {
            uint32 const spell_id = kv.second.spell_id;
            bool const should_have = desired.count(spell_id) > 0;
            bool const has_aura = player->HasAura(spell_id);
            if (should_have && !has_aura)
            {
                player->AddAura(spell_id, player);
                ++applied;
            }
            else if (!should_have && has_aura)
            {
                player->RemoveAurasDueToSpell(spell_id);
                ++removed;
            }
        }

        if (applied || removed)
        {
            LOG_DEBUG("module",
                "[mod-bracket-sets] reeval guid={} class={} spec={} level={} applied={} removed={} desired={}",
                player->GetGUID().GetCounter(), class_id, spec_id, level,
                applied, removed, desired.size());
        }
    }

    void OnPlayerLogout(Player* player)
    {
        if (!player)
            return;
        // Stateless tracking: in-memory state is computed each ReevalAll from
        // the player's own auras + equipped items. Nothing to clean up here.
        // The player's set-bonus auras remain on logout and re-apply on next
        // ReevalAll (which fires from OnPlayerLogin).
    }
}
