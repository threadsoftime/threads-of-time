#ifndef MOD_BRACKET_SETS_PLAYER_SCRIPT_H
#define MOD_BRACKET_SETS_PLAYER_SCRIPT_H

#include "ScriptMgr.h"

namespace BracketSets
{
    class PlayerScript : public ::PlayerScript
    {
    public:
        PlayerScript();

        // Block default set-effect spell application for our managed itemsets.
        // Returning false here means the engine will NOT apply the default
        // bonus spells from item_set.spell1..spell8. We manage bonuses
        // ourselves via BracketSetsManager.
        bool OnPlayerCanApplyEquipSpellsItemSet(Player* player, ItemSetEffect* eff) override;

        // Re-evaluate bonuses on each of these events.
        void OnPlayerLevelChanged(Player* player, uint8 oldlevel) override;
        void OnPlayerAfterSpecSlotChanged(Player* player, uint8 newSlot) override;
        void OnPlayerEquip(Player* player, Item* it, uint8 bag, uint8 slot, bool update) override;
        void OnPlayerUnequip(Player* player, Item* it) override;
        void OnPlayerLogin(Player* player) override;
        void OnPlayerLogout(Player* player) override;
    };

    void RegisterPlayerScript();
}

#endif // MOD_BRACKET_SETS_PLAYER_SCRIPT_H
