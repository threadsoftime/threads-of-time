#ifndef MOD_BRACKET_SETS_COMMAND_SCRIPT_H
#define MOD_BRACKET_SETS_COMMAND_SCRIPT_H

namespace BracketSets
{
    // Registers the .bracketsets GM command tree:
    //
    //   .bracketsets dump [player_name]    — show current state for a player
    //                                        (or the calling GM if no name).
    //                                        Reports: class, spec, level,
    //                                        equipped set counts per itemset,
    //                                        currently-applied marker auras.
    //   .bracketsets reeval [player_name]  — force ReevalAll on a player.
    //                                        Useful after manual DB edits.
    //   .bracketsets reload                — re-read bracket_set_bonus_map
    //                                        from DB (no worldserver restart).
    //
    // Security: SEC_GAMEMASTER.

    void RegisterCommandScript();
}

#endif // MOD_BRACKET_SETS_COMMAND_SCRIPT_H
