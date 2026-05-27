#include "BracketSetsPlayerScript.h"

#include "BracketSetsConfig.h"
#include "BracketSetsManager.h"
#include "BracketSetsRegistry.h"

namespace BracketSets
{
    PlayerScript::PlayerScript()
        : ::PlayerScript("BracketSetsPlayerScript", {
              PLAYERHOOK_CAN_APPLY_EQUIP_SPELLS_ITEM_SET,
              PLAYERHOOK_ON_LEVEL_CHANGED,
              PLAYERHOOK_ON_AFTER_SPEC_SLOT_CHANGED,
              PLAYERHOOK_ON_EQUIP,
              PLAYERHOOK_ON_UNEQUIP_ITEM,
              PLAYERHOOK_ON_LOGIN,
              PLAYERHOOK_ON_LOGOUT
          })
    {
    }

    bool PlayerScript::OnPlayerCanApplyEquipSpellsItemSet(Player* /*player*/, ItemSetEffect* eff)
    {
        if (!GetConfig().Enabled)
            return true;

        if (!eff)
            return true;

        // For sets we own, return false so the engine's default set-effect
        // spell does NOT apply. BracketSetsManager applies our spec-morph
        // bonus aura instead.
        return !IsBracketSet(eff->setid);
    }

    void PlayerScript::OnPlayerLevelChanged(Player* player, uint8 /*oldlevel*/)
    {
        ReevalAll(player);
    }

    void PlayerScript::OnPlayerAfterSpecSlotChanged(Player* player, uint8 /*newSlot*/)
    {
        ReevalAll(player);
    }

    void PlayerScript::OnPlayerEquip(Player* player, Item* /*it*/, uint8 /*bag*/, uint8 /*slot*/, bool /*update*/)
    {
        ReevalAll(player);
    }

    void PlayerScript::OnPlayerUnequip(Player* player, Item* /*it*/)
    {
        ReevalAll(player);
    }

    void PlayerScript::OnPlayerLogin(Player* player)
    {
        ReevalAll(player);
    }

    void PlayerScript::OnPlayerLogout(Player* player)
    {
        BracketSets::OnPlayerLogout(player);
    }

    void RegisterPlayerScript()
    {
        new PlayerScript();
    }
}
