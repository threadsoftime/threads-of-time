#include "WarforgedLootScript.h"
#include "WarforgedAnnouncer.h"
#include "WarforgedManager.h"

// This fork renames the classic AC loot hooks to OnPlayer* form and uses a
// hook-mask opt-in (see modules/mod-bracket-sets/src/BracketSetsPersonalLoot.cpp
// for the canonical pattern). Each hook we override must be listed in the
// constructor's enabledHooks vector or the dispatcher will skip it.
WarforgedLootScript::WarforgedLootScript()
    : PlayerScript("WarforgedLootScript",
                   { PLAYERHOOK_ON_LOOT_ITEM,
                     PLAYERHOOK_ON_QUEST_REWARD_ITEM,
                     PLAYERHOOK_ON_CREATE_ITEM,
                     PLAYERHOOK_ON_GROUP_ROLL_REWARD_ITEM })
{
}

void WarforgedLootScript::OnPlayerLootItem(Player* player, Item* item, uint32 /*count*/, ObjectGuid /*lootguid*/)
{
    auto r = ModWarforged::Manager::RollAndApply(player, item);
    if (r.warforged || r.socket)
        ModWarforged::Announcer::Announce(player, item, r.warforged, r.socket);
}

void WarforgedLootScript::OnPlayerQuestRewardItem(Player* player, Item* item, uint32 /*count*/)
{
    auto r = ModWarforged::Manager::RollAndApply(player, item);
    if (r.warforged || r.socket)
        ModWarforged::Announcer::Announce(player, item, r.warforged, r.socket);
}

void WarforgedLootScript::OnPlayerCreateItem(Player* player, Item* item, uint32 /*count*/)
{
    auto r = ModWarforged::Manager::RollAndApply(player, item);
    if (r.warforged || r.socket)
        ModWarforged::Announcer::Announce(player, item, r.warforged, r.socket);
}

void WarforgedLootScript::OnPlayerGroupRollRewardItem(Player* player, Item* item, uint32 /*count*/, RollVote /*voteType*/, Roll* /*roll*/)
{
    auto r = ModWarforged::Manager::RollAndApply(player, item);
    if (r.warforged || r.socket)
        ModWarforged::Announcer::Announce(player, item, r.warforged, r.socket);
}
