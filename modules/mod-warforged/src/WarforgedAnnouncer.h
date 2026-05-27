#ifndef MOD_WARFORGED_ANNOUNCER_H
#define MOD_WARFORGED_ANNOUNCER_H

class Player;
class Item;

namespace ModWarforged::Announcer
{
    // Sends the orange-prefixed loot proc notification (and the optional sound)
    // through the channel configured in mod_warforged.conf.
    //
    //   announceChannel = 0  -> silent (early-return)
    //   announceChannel = 1  -> whisper the looter only
    //   announceChannel = 2  -> group-aware: broadcast to every group member;
    //                           if the looter is solo, falls back to a whisper.
    //
    // `warforged` / `socket` indicate which proc(s) fired; the call is a no-op
    // when both are false (caller is expected to skip in that case, but we
    // guard internally anyway).
    void Announce(Player* looter, Item* item, bool warforged, bool socket);
}

#endif // MOD_WARFORGED_ANNOUNCER_H
