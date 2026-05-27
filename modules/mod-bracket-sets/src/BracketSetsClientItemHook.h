#ifndef MOD_BRACKET_SETS_CLIENT_ITEM_HOOK_H
#define MOD_BRACKET_SETS_CLIENT_ITEM_HOOK_H

#include "ScriptMgr.h"

class BracketSetsClientItemHook : public PlayerScript {
public:
    BracketSetsClientItemHook();

    // Hook 3a — rewrites itemset_id in outgoing SMSG_ITEM_QUERY_SINGLE_RESPONSE.
    // The core invokes this just before serializing the ItemSet field; we mutate
    // `itemSet` in place. See spec §5.3.
    void OnPlayerBuildItemQueryResponse(Player* player, ItemTemplate const* proto, uint32& itemSet) override;

    // Hook 3b — full respec: push fresh item-query responses for all owned
    // Bracket 1 pieces so the client cache reflects the player's new spec.
    void OnPlayerTalentsReset(Player* player, bool noCost) override;

    // Hook 3b — dual-spec swap: same as full respec.
    void OnPlayerAfterSpecSlotChanged(Player* player, uint8 newSlot) override;
};

#endif  // MOD_BRACKET_SETS_CLIENT_ITEM_HOOK_H
