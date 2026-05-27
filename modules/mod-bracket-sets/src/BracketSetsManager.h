#ifndef MOD_BRACKET_SETS_MANAGER_H
#define MOD_BRACKET_SETS_MANAGER_H

#include "Common.h"
#include "BracketSetsItemsetResolver.h"

class Player;

namespace BracketSets
{
    // Per-player apply / remove logic. Walks the player's equipped items,
    // counts pieces per managed itemset, looks up the spec-morph bonus, and
    // applies or removes the corresponding aura. Tracks active auras per
    // player so removal at level-out-of-bracket is clean.
    //
    // v0 scaffold: all methods are no-ops. Step 5 of the implementation plan
    // fills these in.

    void ReevalAll(Player* player);
    void OnPlayerLogout(Player* player);

    // Itemset map — loaded once at world boot from bracket_set_itemset_map.
    // Loaded alongside LoadBonusMap() in BracketSetsWorldScript::OnStartup().
    void LoadItemsetMap();

    // Returns the patched itemset_id for (class, spec, bracket).
    // Returns 0 if no mapping exists (e.g. Death Knight for bracket 1).
    uint32 ResolveItemset(uint8 classId, uint8 specId, uint8 bracketId);

    // Dominant talent-tab index (1-based) for the player's active spec slot.
    // Ties resolve to the lower index; 0-talent characters return 1.
    // Use this instead of Player::GetActiveSpec() — that returns the
    // dual-spec SLOT (0 or 1), not the talent build.
    uint8 DetermineSpec(Player* player);
}

#endif // MOD_BRACKET_SETS_MANAGER_H
