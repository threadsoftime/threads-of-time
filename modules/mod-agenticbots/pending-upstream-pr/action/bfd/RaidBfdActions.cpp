// SPDX-License-Identifier: GPL-2.0-or-later
//
// NOTE: These actions are not yet wired into mod-agenticbots's startup. They depend
// on mod-playerbots exposing 4 custom-context registration methods — see
// docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-foundation.md Task 18.
// Once the upstream PR lands (or a Heimdal-internal patch is shipped), the
// AgenticbotsRegistration.cpp glue will register these actions via RaidBfdActionContext.

#include "RaidBfdActions.h"

#include "GenericSpellActions.h"
#include "MovementActions.h"
#include "Playerbots.h"

namespace
{
    constexpr uint32 NPC_GHAMOO_RA             = 4887;
    constexpr uint32 NPC_BLACKFATHOM_SEA_WITCH = 4805;
    constexpr uint32 NPC_AKUMAI                = 4829;
    constexpr float  AKUMAI_SAFE_SPREAD        = 9.0f;
}

// ---- V1: delegate-to-class cleanse/dispel/interrupt --------------------------
//
// PlayerbotAI exposes DoSpecificAction(name, event, silent) which dispatches
// through all engines and tries to execute an action by name. The class-
// specific action contexts (Mage's "remove lesser curse", Druid's "remove
// curse", Priest's "dispel magic", etc.) register their own action names and
// the engine will fan out across them. We try a short list of known names;
// the first one that succeeds wins. This means a non-dispelling class (rogue,
// warrior) simply no-ops.

bool BfdCleanseCurseAction::Execute(Event event)
{
    // Try the class-specific decurse action names in order.
    static const std::vector<std::string> kNames = {
        "remove curse",          // druid
        "remove lesser curse",   // mage
    };
    for (auto const& name : kNames)
    {
        if (botAI->DoSpecificAction(name, event, true))
            return true;
    }
    return false;
}

bool BfdDispelMagicAction::Execute(Event event)
{
    static const std::vector<std::string> kNames = {
        "dispel magic",          // priest
        "cleanse",               // paladin (cleanses magic + poison + disease)
    };
    for (auto const& name : kNames)
    {
        if (botAI->DoSpecificAction(name, event, true))
            return true;
    }
    return false;
}

bool BfdInterruptBossAction::Execute(Event event)
{
    static const std::vector<std::string> kNames = {
        "pummel",                // warrior arms/fury
        "shield bash",           // warrior protection
        "kick",                  // rogue
        "counterspell",          // mage
        "earth shock",           // shaman
    };
    for (auto const& name : kNames)
    {
        if (botAI->DoSpecificAction(name, event, true))
            return true;
    }
    return false;
}

// ---- V2: spread on Aku'mai whirlwind -----------------------------------------

bool BfdSpreadFromAkumaiAction::Execute(Event /*event*/)
{
    // Find Aku'mai, move outside melee range.
    Unit* akumai = nullptr;
    GuidVector targets = AI_VALUE(GuidVector, "possible targets");
    for (ObjectGuid guid : targets)
    {
        Creature* c = botAI->GetCreature(guid);
        if (c && c->IsAlive() && c->GetEntry() == NPC_AKUMAI)
        {
            akumai = c;
            break;
        }
    }
    if (!akumai)
        return false;

    // Only melee bots need to back away; ranged are already far enough.
    if (bot->GetDistance(akumai) > AKUMAI_SAFE_SPREAD)
        return false;

    // Vector away from Aku'mai.
    float angle = akumai->GetAngle(bot);
    float dx = cos(angle) * AKUMAI_SAFE_SPREAD;
    float dy = sin(angle) * AKUMAI_SAFE_SPREAD;
    float x = akumai->GetPositionX() + dx;
    float y = akumai->GetPositionY() + dy;
    float z = bot->GetPositionZ();

    return MoveTo(bot->GetMapId(), x, y, z, false, false, false, false,
                  MovementPriority::MOVEMENT_COMBAT);
}

// ---- V2: focus-burn Sea Witch adds -------------------------------------------

bool BfdKillSeaWitchAction::Execute(Event /*event*/)
{
    Unit* currentTarget = AI_VALUE(Unit*, "current target");
    // Already attacking a sea witch — don't churn the target.
    if (currentTarget && currentTarget->GetEntry() == NPC_BLACKFATHOM_SEA_WITCH)
        return false;

    GuidVector targets = AI_VALUE(GuidVector, "possible targets");
    for (ObjectGuid guid : targets)
    {
        Creature* c = botAI->GetCreature(guid);
        if (!c || !c->IsAlive() || !c->IsInWorld())
            continue;
        if (c->GetEntry() == NPC_BLACKFATHOM_SEA_WITCH)
            return Attack(c);
    }
    return false;
}

// ---- V2: tank swap on Ghamoo-ra Sunder stacks --------------------------------

bool BfdTankSwapGhamooAction::Execute(Event event)
{
    // Find Ghamoo-ra and taunt it onto me. The classed "taunt" action
    // exists on warrior, paladin, druid (bear/cat), DK — DoSpecificAction
    // will fan out and find the right one for the class.
    GuidVector targets = AI_VALUE(GuidVector, "possible targets");
    for (ObjectGuid guid : targets)
    {
        Creature* c = botAI->GetCreature(guid);
        if (!c || !c->IsAlive() || c->GetEntry() != NPC_GHAMOO_RA)
            continue;

        // Set Ghamoo-ra as the current target before triggering taunt.
        bot->SetSelection(c->GetGUID());
        if (botAI->DoSpecificAction("taunt", event, true))
            return true;
        if (botAI->DoSpecificAction("growl", event, true))   // druid bear
            return true;
        if (botAI->DoSpecificAction("hand of reckoning", event, true))   // paladin
            return true;
    }
    return false;
}
