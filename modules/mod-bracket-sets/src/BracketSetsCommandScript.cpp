#include "BracketSetsCommandScript.h"

#include "BracketSetsConfig.h"
#include "BracketSetsManager.h"
#include "BracketSetsPersonalLoot.h"
#include "BracketSetsRegistry.h"
#include "Chat.h"
#include "Item.h"
#include "ItemTemplate.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "ScriptMgr.h"
#include "SpellInfo.h"
#include "SpellMgr.h"

#include <cstdlib>
#include <cstring>
#include <map>
#include <string>

namespace
{
    using namespace Acore::ChatCommands;

    // Resolves an arg to a Player*: the named character if provided and online,
    // otherwise the caller's selected unit, otherwise the caller themselves.
    Player* ResolveTarget(ChatHandler* handler, char const* args)
    {
        if (args && *args)
        {
            std::string name = args;
            if (Player* p = ObjectAccessor::FindPlayerByName(name, false))
                return p;
            handler->PSendSysMessage("|cffff0000Player '{}' not online.|r", name);
            return nullptr;
        }
        // Fallback: caller's selection if a player; else caller.
        if (handler->GetSession())
        {
            if (Player* sel = handler->getSelectedPlayerOrSelf())
                return sel;
        }
        return nullptr;
    }

    static bool HandleDump(ChatHandler* handler, char const* args)
    {
        if (!BracketSets::GetConfig().Enabled)
        {
            handler->PSendSysMessage("|cffff0000mod-bracket-sets is disabled in server config|r");
            return true;
        }

        Player* p = ResolveTarget(handler, args);
        if (!p)
        {
            handler->PSendSysMessage("usage: .bracketsets dump [player_name]");
            return true;
        }

        // Count equipped set pieces per managed itemset.
        std::map<uint32, uint8> counts;
        for (uint8 slot = EQUIPMENT_SLOT_START; slot < EQUIPMENT_SLOT_END; ++slot)
        {
            Item* item = p->GetItemByPos(INVENTORY_SLOT_BAG_0, slot);
            if (!item)
                continue;
            ItemTemplate const* proto = item->GetTemplate();
            if (!proto || !BracketSets::IsBracketSet(proto->ItemSet))
                continue;
            ++counts[proto->ItemSet];
        }

        handler->PSendSysMessage("=== bracketsets dump: {} ===", p->GetName());
        handler->PSendSysMessage(" class={}  level={}  guid={}",
            uint32(p->getClass()), uint32(p->GetLevel()), p->GetGUID().GetCounter());

        if (counts.empty())
        {
            handler->PSendSysMessage(" no bracket set pieces equipped");
        }
        else
        {
            for (auto const& kv : counts)
                handler->PSendSysMessage(" itemset {} -> {} pieces equipped", kv.first, uint32(kv.second));
        }

        // Show currently-applied marker auras (managed by us).
        size_t found = 0;
        for (auto const& kv : BracketSets::Bonuses())
        {
            uint32 spell_id = kv.second.spell_id;
            if (!p->HasAura(spell_id))
                continue;
            ++found;
            handler->PSendSysMessage(
                " active aura: spell={}  '{}'  (itemset={} threshold={} class={} spec={})",
                spell_id, kv.second.display_name,
                kv.first.itemset_id, uint32(kv.first.threshold),
                uint32(kv.first.class_id), uint32(kv.first.spec_id));
        }
        if (found == 0)
            handler->PSendSysMessage(" no bracket-sets marker auras currently active");

        return true;
    }

    static bool HandleReeval(ChatHandler* handler, char const* args)
    {
        if (!BracketSets::GetConfig().Enabled)
        {
            handler->PSendSysMessage("|cffff0000mod-bracket-sets is disabled in server config|r");
            return true;
        }

        Player* p = ResolveTarget(handler, args);
        if (!p)
        {
            handler->PSendSysMessage("usage: .bracketsets reeval [player_name]");
            return true;
        }

        BracketSets::ReevalAll(p);
        handler->PSendSysMessage("bracketsets: forced ReevalAll on {}", p->GetName());
        return true;
    }

    static bool HandleReload(ChatHandler* handler, char const* /*args*/)
    {
        BracketSets::ReloadBonusMap();
        BracketSets::ReloadPersonalLootMap();
        handler->PSendSysMessage(
            "bracketsets: reloaded bonus map ({} rows) + personal loot ({} boss(es), {} rows)",
            BracketSets::BonusCount(),
            BracketSets::PersonalLootBossCount(),
            BracketSets::PersonalLootRowCount());
        return true;
    }

    static bool HandleKillTest(ChatHandler* handler, char const* args)
    {
        if (!args || !*args)
        {
            handler->PSendSysMessage(
                "usage: .bracketsets killtest <boss_entry> [<player_name>]");
            return true;
        }
        std::string buf = args;
        char* rest = nullptr;
        char* boss_str = std::strtok(buf.data(), " ");
        char* name_str = std::strtok(nullptr, " ");
        (void)rest;
        if (!boss_str)
        {
            handler->PSendSysMessage("usage: .bracketsets killtest <boss_entry> [<player_name>]");
            return true;
        }
        uint32 const boss_entry = uint32(std::atoi(boss_str));
        Player* p = ResolveTarget(handler, name_str);
        if (!p)
        {
            handler->PSendSysMessage("usage: .bracketsets killtest <boss_entry> <player_name>");
            return true;
        }

        std::size_t const drops = BracketSets::SimulateBossKill(boss_entry, p);
        handler->PSendSysMessage(
            "bracketsets: simulated kill of boss {} for {} -> {} drop(s)",
            boss_entry, p->GetName(), drops);
        return true;
    }

    // Prints a spell's effect table so we can verify (EffectIndex, ApplyAuraName)
    // pairs match what our AuraScript::Register() bindings declare.
    static bool HandleDiag(ChatHandler* handler, char const* args)
    {
        if (!args || !*args)
        {
            handler->PSendSysMessage("usage: .bracketsets diag <spell_id>");
            return true;
        }
        uint32 spell_id = uint32(std::atoi(args));
        SpellInfo const* info = sSpellMgr->GetSpellInfo(spell_id);
        if (!info)
        {
            handler->PSendSysMessage(
                "|cffff0000spell {} does not exist in Spell.dbc|r", spell_id);
            return true;
        }
        char const* name = info->SpellName[0] ? info->SpellName[0] : "?";
        handler->PSendSysMessage(
            "=== bracketsets diag: spell {} '{}' ===", spell_id, name);
        for (uint8 i = 0; i < MAX_SPELL_EFFECTS; ++i)
        {
            SpellEffectInfo const& eff = info->Effects[i];
            if (eff.Effect == 0 && eff.ApplyAuraName == SPELL_AURA_NONE)
                continue;
            handler->PSendSysMessage(
                " EFFECT_{}  Effect={}  ApplyAuraName={}  Amplitude={}",
                uint32(i), uint32(eff.Effect),
                uint32(eff.ApplyAuraName), eff.Amplitude);
        }
        return true;
    }

    class BracketSetsCommandScript : public CommandScript
    {
    public:
        BracketSetsCommandScript() : CommandScript("BracketSetsCommandScript") {}

        ChatCommandTable GetCommands() const override
        {
            static ChatCommandTable bracketsetsCommandTable =
            {
                { "dump",     HandleDump,     SEC_GAMEMASTER, Console::Yes },
                { "reeval",   HandleReeval,   SEC_GAMEMASTER, Console::Yes },
                { "reload",   HandleReload,   SEC_GAMEMASTER, Console::Yes },
                { "diag",     HandleDiag,     SEC_GAMEMASTER, Console::Yes },
                { "killtest", HandleKillTest, SEC_GAMEMASTER, Console::Yes },
            };

            static ChatCommandTable commandTable =
            {
                { "bracketsets", bracketsetsCommandTable },
            };

            return commandTable;
        }
    };
}

namespace BracketSets
{
    void RegisterCommandScript()
    {
        new ::BracketSetsCommandScript();
    }
}
