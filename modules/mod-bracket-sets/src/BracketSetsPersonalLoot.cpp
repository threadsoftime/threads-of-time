#include "BracketSetsPersonalLoot.h"

#include "Creature.h"
#include "DatabaseEnv.h"
#include "Field.h"
#include "Group.h"
#include "GroupReference.h"
#include "Item.h"
#include "Log.h"
#include "Mail.h"
#include "Player.h"
#include "QueryResult.h"
#include "Random.h"
#include "ScriptMgr.h"

#include <cstdint>
#include <unordered_map>
#include <vector>

namespace BracketSets
{
    namespace
    {
        struct LootRow
        {
            uint32 item_id;
            float  chance;
        };

        using LootPool = std::vector<LootRow>;
        std::unordered_map<uint32, LootPool> gPersonalLoot;
        std::size_t gRowCount = 0;

        void MailItem(Player* player, uint32 item_id)
        {
            CharacterDatabaseTransaction trans = CharacterDatabase.BeginTransaction();
            if (Item* item = Item::CreateItem(item_id, 1, player))
            {
                item->SaveToDB(trans);
                MailDraft("Bracket 1 Loot",
                          "Your bag was full when this dropped from a Blackfathom Deeps boss.")
                    .AddItem(item)
                    .SendMailTo(trans, MailReceiver(player),
                                MailSender(MAIL_NORMAL, 0));
            }
            CharacterDatabase.CommitTransaction(trans);
        }

        void GiveItem(Player* player, uint32 item_id)
        {
            ItemPosCountVec dest;
            InventoryResult const msg = player->CanStoreNewItem(
                NULL_BAG, NULL_SLOT, dest, item_id, 1);
            if (msg == EQUIP_ERR_OK)
            {
                if (Item* item = player->StoreNewItem(dest, item_id, true))
                    player->SendNewItem(item, 1, true, false);
            }
            else
            {
                MailItem(player, item_id);
            }
        }

        // Loot range mirrors the stock corpse-loot radius so we don't reward
        // players who AFK'd outside the boss room.
        constexpr float LOOT_RANGE = 100.f;

        bool Eligible(Player* p, Creature* victim)
        {
            return p && p->IsInMap(victim) && p->IsAlive()
                && p->GetDistance(victim) < LOOT_RANGE;
        }

        void CollectParticipants(Player* killer, Creature* victim,
                                 std::vector<Player*>& out)
        {
            if (Group* group = killer->GetGroup())
            {
                for (GroupReference* ref = group->GetFirstMember();
                     ref; ref = ref->next())
                {
                    if (Player* p = ref->GetSource())
                        if (Eligible(p, victim))
                            out.push_back(p);
                }
            }
            else if (Eligible(killer, victim))
            {
                out.push_back(killer);
            }
        }

        class BracketSetsPersonalLootScript : public PlayerScript
        {
        public:
            BracketSetsPersonalLootScript()
                : PlayerScript("BracketSetsPersonalLootScript",
                               { PLAYERHOOK_ON_CREATURE_KILL })
            {
            }

            void OnPlayerCreatureKill(Player* killer, Creature* killed) override
            {
                if (!killer || !killed)
                    return;

                auto it = gPersonalLoot.find(killed->GetEntry());
                if (it == gPersonalLoot.end())
                    return;

                LootPool const& pool = it->second;

                std::vector<Player*> participants;
                participants.reserve(10);
                CollectParticipants(killer, killed, participants);
                if (participants.empty())
                    return;

                uint32 total_drops = 0;
                for (Player* p : participants)
                {
                    for (LootRow const& row : pool)
                    {
                        if (roll_chance_f(row.chance))
                        {
                            GiveItem(p, row.item_id);
                            ++total_drops;
                        }
                    }
                }

                LOG_DEBUG("module",
                    "[bracket-sets-personal-loot] boss={} entry={} participants={} drops={}",
                    killed->GetName(), killed->GetEntry(),
                    participants.size(), total_drops);
            }
        };
    }

    void LoadPersonalLootMap()
    {
        gPersonalLoot.clear();
        gRowCount = 0;

        QueryResult result = WorldDatabase.Query(
            "SELECT `boss_entry`, `item_id`, `chance` FROM `bracket_personal_loot`");
        if (!result)
        {
            LOG_WARN("server.loading",
                     "[mod-bracket-sets] bracket_personal_loot is empty/missing — personal loot disabled");
            return;
        }

        do {
            Field* f = result->Fetch();
            uint32 const boss   = f[0].Get<uint32>();
            uint32 const item   = f[1].Get<uint32>();
            float  const chance = f[2].Get<float>();
            gPersonalLoot[boss].push_back({item, chance});
            ++gRowCount;
        } while (result->NextRow());

        LOG_INFO("server.loading",
                 "[mod-bracket-sets] personal loot map: {} boss(es), {} row(s)",
                 gPersonalLoot.size(), gRowCount);
    }

    void ReloadPersonalLootMap()
    {
        LoadPersonalLootMap();
    }

    std::size_t PersonalLootBossCount() { return gPersonalLoot.size(); }
    std::size_t PersonalLootRowCount()  { return gRowCount; }

    std::size_t SimulateBossKill(std::uint32_t boss_entry, Player* killer)
    {
        if (!killer)
            return 0;
        auto it = gPersonalLoot.find(boss_entry);
        if (it == gPersonalLoot.end())
            return 0;

        LootPool const& pool = it->second;

        std::vector<Player*> participants;
        if (Group* group = killer->GetGroup())
        {
            for (GroupReference* ref = group->GetFirstMember();
                 ref; ref = ref->next())
            {
                if (Player* p = ref->GetSource())
                    if (p->IsAlive() && p->IsInWorld())
                        participants.push_back(p);
            }
        }
        else
        {
            participants.push_back(killer);
        }

        std::size_t total_drops = 0;
        for (Player* p : participants)
        {
            for (LootRow const& row : pool)
            {
                if (roll_chance_f(row.chance))
                {
                    GiveItem(p, row.item_id);
                    ++total_drops;
                }
            }
        }
        return total_drops;
    }

    void RegisterPersonalLootScript()
    {
        new BracketSetsPersonalLootScript();
    }
}
