#ifndef MOD_WARFORGED_LOOT_SCRIPT_H
#define MOD_WARFORGED_LOOT_SCRIPT_H

#include "ScriptMgr.h"

class WarforgedLootScript : public PlayerScript
{
public:
    WarforgedLootScript();

    // Note: this fork renames the classic AC hooks to OnPlayer* form
    // (see src/server/game/Scripting/ScriptDefines/PlayerScript.h).
    // Hook-mask opt-in (constructor below) is matched here.
    void OnPlayerLootItem(Player* player, Item* item, uint32 /*count*/, ObjectGuid /*lootguid*/) override;
    void OnPlayerQuestRewardItem(Player* player, Item* item, uint32 /*count*/) override;
    void OnPlayerCreateItem(Player* player, Item* item, uint32 /*count*/) override;
    void OnPlayerGroupRollRewardItem(Player* player, Item* item, uint32 /*count*/, RollVote /*voteType*/, Roll* /*roll*/) override;
};

#endif // MOD_WARFORGED_LOOT_SCRIPT_H
