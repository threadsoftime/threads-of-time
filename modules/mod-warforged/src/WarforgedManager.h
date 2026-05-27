#ifndef MOD_WARFORGED_MANAGER_H
#define MOD_WARFORGED_MANAGER_H

class Item;
class Player;

namespace ModWarforged::Manager
{
    // Returns (warforged_applied, socket_applied) so callers know what to announce.
    struct RollResult { bool warforged = false; bool socket = false; };

    // Eligibility: quality in [Min,Max], not quest token, not stackable.
    bool IsEligible(Item* item);

    // The full pipeline: roll, pick enchant, apply slots, mark item dirty.
    // Returns what was applied. Does NOT announce — caller handles announce.
    RollResult RollAndApply(Player* player, Item* item);
}

#endif
