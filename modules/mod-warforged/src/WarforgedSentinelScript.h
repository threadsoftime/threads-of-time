#ifndef MOD_WARFORGED_SENTINEL_SCRIPT_H
#define MOD_WARFORGED_SENTINEL_SCRIPT_H

#include "ObjectGuid.h"
#include "ScriptMgr.h"

#include <cstdint>
#include <string>
#include <unordered_map>

// PING/PONG version sentinel for mod-warforged's client patch (patch-W.MPQ).
//
// Hook surface (validated against this fork's PlayerScript.h on 2026-05-24):
//   - OnPlayerLogin(Player*)                                  -> PLAYERHOOK_ON_LOGIN
//   - OnPlayerLogout(Player*)                                 -> PLAYERHOOK_ON_LOGOUT
//   - OnPlayerBeforeSendChatMessage(Player*, uint32&, uint32&, std::string&)
//                                                             -> PLAYERHOOK_ON_BEFORE_SEND_CHAT_MESSAGE
//
// The plan referred to "OnPlayerChat" but that hook does not exist on this
// fork; the canonical incoming-chat surface is OnPlayerBeforeSendChatMessage.
// See pre-flight rule kb_6950a902 §1 (API drift).
class WarforgedSentinelScript : public PlayerScript
{
public:
    WarforgedSentinelScript();

    void OnPlayerLogin(Player* player) override;
    void OnPlayerLogout(Player* player) override;

    // Intercept ALL outgoing chat from the player; we filter for addon-channel
    // messages with the "WF" prefix (== our PONG) inside the body.
    void OnPlayerBeforeSendChatMessage(Player* player, uint32& type, uint32& lang, std::string& msg) override;

    // Per-player state: whether a PONG has been received this login session.
    // Cleared on OnPlayerLogin (fresh pending) and OnPlayerLogout.
    static bool HasRespondedPong(ObjectGuid playerGuid);

private:
    // GUID raw-value -> seen_pong flag (true once a valid PONG was received).
    // Static so the delayed timeout event can reach it without holding a
    // pointer to the script instance (the script lives forever; the event
    // closure does not need ownership).
    static std::unordered_map<uint64_t, bool> sPongReceived;
};

#endif // MOD_WARFORGED_SENTINEL_SCRIPT_H
