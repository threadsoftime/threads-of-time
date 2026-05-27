#include "WarforgedAnnouncer.h"
#include "WarforgedConfig.h"

#include "Chat.h"
#include "Group.h"
#include "GroupReference.h"
#include "Item.h"
#include "ItemTemplate.h"
#include "Opcodes.h"
#include "Player.h"
#include "SharedDefines.h"   // ItemQualityColors[]
#include "WorldPacket.h"
#include "WorldSession.h"    // ChatHandler ctor takes WorldSession*

#include <sstream>
#include <string>

namespace ModWarforged::Announcer
{
    namespace
    {
        // This fork does NOT expose Item::GetItemLink(); we mirror the canonical
        // link-builder pattern from cs_character.cpp / PlayerStorage.cpp instead.
        //
        // Resulting format:
        //   |c<argb_hex>|Hitem:<entry>:0:0:0:0:0:0:0:0:0|h[<Name1><tag>]|h|r
        //
        // <tag> is one of:
        //   " (Warforged)"            warforged-only
        //   " (Socketed)"             socket-only
        //   " (Warforged, Socketed)"  both
        std::string BuildProcTaggedLink(Item* item, bool warforged, bool socket)
        {
            ItemTemplate const* tpl = item->GetTemplate();

            std::string tag;
            if (warforged && socket) tag = " (Warforged, Socketed)";
            else if (warforged)      tag = " (Warforged)";
            else if (socket)         tag = " (Socketed)";

            std::ostringstream link;
            link << "|c"
                 << std::hex << ItemQualityColors[tpl->Quality] << std::dec
                 << "|Hitem:" << tpl->ItemId << ":0:0:0:0:0:0:0:0:0|h["
                 << tpl->Name1 << tag
                 << "]|h|r";
            return link.str();
        }

        void PlayProcSound(Player* p)
        {
            if (!gConfig.playLootSound) return;
            if (!p) return;

            WorldPacket data(SMSG_PLAY_SOUND, 4);
            data << uint32(gConfig.lootSoundId);
            p->SendDirectMessage(&data);
        }
    }

    void Announce(Player* looter, Item* item, bool warforged, bool socket)
    {
        if (gConfig.announceChannel == 0) return;
        if (!looter || !item) return;
        if (!warforged && !socket) return;

        std::string prefix;
        if (warforged && socket) prefix = "|cffff8000Warforged + Socketed!|r ";
        else if (warforged)      prefix = "|cffff8000Warforged!|r ";
        else if (socket)         prefix = "|cffffd200Bonus Socket!|r ";

        std::string link = BuildProcTaggedLink(item, warforged, socket);
        std::string msg  = prefix + looter->GetName() + " looted " + link;

        switch (gConfig.announceChannel)
        {
            case 1:  // whisper looter only
                ChatHandler(looter->GetSession()).PSendSysMessage("%s", msg.c_str());
                break;

            case 2:  // group-aware
                if (Group* g = looter->GetGroup())
                {
                    for (GroupReference* itr = g->GetFirstMember(); itr != nullptr; itr = itr->next())
                    {
                        if (Player* member = itr->GetSource())
                            ChatHandler(member->GetSession()).PSendSysMessage("%s", msg.c_str());
                    }
                }
                else
                {
                    ChatHandler(looter->GetSession()).PSendSysMessage("%s", msg.c_str());
                }
                break;

            default:
                // Unknown channel value — treat as off; avoid silent fallthrough surprises.
                return;
        }

        PlayProcSound(looter);
    }
}
