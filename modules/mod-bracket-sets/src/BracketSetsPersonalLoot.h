#ifndef MOD_BRACKET_SETS_PERSONAL_LOOT_H
#define MOD_BRACKET_SETS_PERSONAL_LOOT_H

#include <cstddef>
#include <cstdint>

class Player;

namespace BracketSets
{
    void LoadPersonalLootMap();
    void ReloadPersonalLootMap();
    std::size_t PersonalLootBossCount();
    std::size_t PersonalLootRowCount();
    void RegisterPersonalLootScript();

    // Test helper -- runs the exact same per-player roll + GiveItem path as
    // the OnPlayerCreatureKill hook, but for a synthesized "kill" by the
    // given player against the given boss entry. Returns the number of
    // items delivered (across all eligible group members, or just the
    // single killer if ungrouped). Used by `.bracketsets killtest`.
    std::size_t SimulateBossKill(std::uint32_t boss_entry, Player* killer);
}

#endif
