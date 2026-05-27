#include "WarforgedSentinelScript.h"

#include "WarforgedConfig.h"

#include "Chat.h"
#include "Log.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "SharedDefines.h"   // CHAT_MSG_ADDON, LANG_ADDON
#include "WorldPacket.h"
#include "WorldSession.h"

#include <utility>

std::unordered_map<uint64_t, bool> WarforgedSentinelScript::sPongReceived;

namespace
{
    constexpr char const* WF_PREFIX     = "WF";
    constexpr char const* WF_PING_BODY  = "PING:1.0.0";
    constexpr char const* WF_PONG_TAG   = "PONG:";
    constexpr uint32      WF_PING_DELAY_MS    = 3000;
    constexpr uint32      WF_TIMEOUT_AFTER_MS = 5000;  // total: ping at 3s, timeout at 8s

    // Build "<prefix>\t<message>" body and ship as CHAT_MSG_WHISPER / LANG_ADDON.
    //
    // Server-to-client addon dispatch (3.3.5a) uses SMSG_MESSAGECHAT with
    // chatType=CHAT_MSG_WHISPER and language=LANG_ADDON (0xFFFFFFFF). The
    // 3.3.5a client demuxes addon messages via the LANG_ADDON marker, not
    // via CHAT_MSG_ADDON. AC's own AddonChannelCommandHandler::Send uses this
    // exact pattern (Chat.cpp:1102), so we mirror it.
    void SendAddonPing(Player* player)
    {
        if (!player || !player->GetSession())
            return;

        std::string body = std::string(WF_PREFIX) + "\t" + WF_PING_BODY;

        WorldPacket data;
        ChatHandler::BuildChatPacket(
            data,
            CHAT_MSG_WHISPER,
            LANG_ADDON,
            player,   // sender
            player,   // receiver (self)
            body);
        player->GetSession()->SendPacket(&data);

        LOG_DEBUG("server.loading",
                  "[mod-warforged] Sentinel: PING sent to {} (GUID {})",
                  player->GetName(), player->GetGUID().ToString());
    }

    // Delayed event: send the PING. We resolve the player by GUID at fire
    // time rather than capturing the Player* -- a logout in the 3s window
    // would otherwise leave us with a dangling pointer.
    class SendPingEvent : public BasicEvent
    {
    public:
        explicit SendPingEvent(ObjectGuid guid) : _guid(guid) {}

        bool Execute(uint64 /*e_time*/, uint32 /*p_time*/) override
        {
            if (Player* p = ObjectAccessor::FindPlayer(_guid))
                SendAddonPing(p);
            return true;
        }

    private:
        ObjectGuid _guid;
    };

    // Delayed event that fires WF_PING_DELAY_MS + WF_TIMEOUT_AFTER_MS after login.
    // If sPongReceived is still false, the client did not respond -> client is
    // likely missing patch-W.MPQ.
    class CheckPongTimeoutEvent : public BasicEvent
    {
    public:
        CheckPongTimeoutEvent(ObjectGuid guid, std::string name)
            : _guid(guid), _playerName(std::move(name)) {}

        bool Execute(uint64 /*e_time*/, uint32 /*p_time*/) override
        {
            if (WarforgedSentinelScript::HasRespondedPong(_guid))
                return true;  // PONG already received, nothing to do

            LOG_INFO("server.loading",
                     "[mod-warforged] Sentinel: player {} (GUID {}) did not "
                     "respond to PING within {}ms -- client may be missing patch-W.MPQ",
                     _playerName, _guid.ToString(), WF_TIMEOUT_AFTER_MS);

            if (!ModWarforged::gConfig.warnMissingPatch)
                return true;

            // Player may have logged out during the wait window. Re-resolve.
            Player* p = ObjectAccessor::FindPlayer(_guid);
            if (!p || !p->GetSession())
                return true;

            ChatHandler(p->GetSession()).PSendSysMessage(
                "Your client appears to be missing the Warforged patch -- "
                "Warforged tooltips may not render correctly. "
                "Ask in #help if you need the patch file.");
            return true;
        }

    private:
        ObjectGuid  _guid;
        std::string _playerName;
    };
}  // namespace

// Constructor must list every hook we override in enabledHooks, otherwise the
// dispatcher skips them on this fork (canonical pattern: mod-bracket-sets/src/
// BracketSetsPlayerScript.cpp).
WarforgedSentinelScript::WarforgedSentinelScript()
    : PlayerScript("WarforgedSentinelScript",
                   { PLAYERHOOK_ON_LOGIN,
                     PLAYERHOOK_ON_LOGOUT,
                     PLAYERHOOK_ON_BEFORE_SEND_CHAT_MESSAGE })
{
}

bool WarforgedSentinelScript::HasRespondedPong(ObjectGuid playerGuid)
{
    auto it = sPongReceived.find(playerGuid.GetRawValue());
    return it != sPongReceived.end() && it->second;
}

void WarforgedSentinelScript::OnPlayerLogin(Player* player)
{
    if (!player)
        return;

    // Reset pong-pending state for this GUID.
    sPongReceived[player->GetGUID().GetRawValue()] = false;

    ObjectGuid guid = player->GetGUID();
    std::string name = player->GetName();

    // Schedule the PING ~3s after login (client needs time to load FrameXML).
    // SendPingEvent resolves Player by GUID at fire time, so a logout in the
    // 3s window can't leave us with a stale pointer.
    player->m_Events.AddEvent(
        new SendPingEvent(guid),
        player->m_Events.CalculateTime(WF_PING_DELAY_MS));

    // Schedule the timeout check (3s + 5s = 8s after login).
    player->m_Events.AddEvent(
        new CheckPongTimeoutEvent(guid, std::move(name)),
        player->m_Events.CalculateTime(WF_PING_DELAY_MS + WF_TIMEOUT_AFTER_MS));
}

void WarforgedSentinelScript::OnPlayerLogout(Player* player)
{
    if (player)
        sPongReceived.erase(player->GetGUID().GetRawValue());
}

void WarforgedSentinelScript::OnPlayerBeforeSendChatMessage(Player* player,
                                                            uint32& type,
                                                            uint32& lang,
                                                            std::string& msg)
{
    if (!player)
        return;

    // We only care about addon-channel messages -- LANG_ADDON marker is the
    // 3.3.5a-correct test; CHAT_MSG_ADDON arrives as a WHISPER on this fork
    // when LANG_ADDON is set.
    if (lang != LANG_ADDON && type != CHAT_MSG_ADDON)
        return;

    // Body format: "WF\tPONG:<version>"
    if (msg.size() < 4)
        return;
    if (msg.compare(0, 2, WF_PREFIX) != 0 || msg[2] != '\t')
        return;

    std::string body = msg.substr(3);
    if (body.compare(0, 5, WF_PONG_TAG) != 0)
        return;

    std::string version = body.substr(5);

    sPongReceived[player->GetGUID().GetRawValue()] = true;
    LOG_DEBUG("server.loading",
              "[mod-warforged] Sentinel: PONG received from {} -- patch version {}",
              player->GetName(), version);
}
