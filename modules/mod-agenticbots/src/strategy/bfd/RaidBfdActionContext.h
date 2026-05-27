// SPDX-License-Identifier: GPL-2.0-or-later
//
// NOTE: This context is not yet wired into mod-agenticbots's startup. It depends
// on mod-playerbots exposing 4 custom-context registration methods — see
// docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-foundation.md Task 18.
// Once the upstream PR lands (or a Heimdal-internal patch is shipped), the
// AgenticbotsRegistration.cpp glue will register this context.

#ifndef _PLAYERBOT_RAIDBFDACTIONCONTEXT_H_
#define _PLAYERBOT_RAIDBFDACTIONCONTEXT_H_

#include "Action.h"
#include "NamedObjectContext.h"
#include "RaidBfdActions.h"

class RaidBfdActionContext : public NamedObjectContext<Action>
{
public:
    RaidBfdActionContext()
    {
        creators["bfd cleanse curse"]    = &RaidBfdActionContext::cleanse_curse;
        creators["bfd dispel magic"]     = &RaidBfdActionContext::dispel_magic;
        creators["bfd interrupt boss"]   = &RaidBfdActionContext::interrupt_boss;
        creators["bfd spread from akumai"] = &RaidBfdActionContext::spread_akumai;
        creators["bfd kill sea witch"]   = &RaidBfdActionContext::kill_sea_witch;
        creators["bfd tank swap ghamoo"] = &RaidBfdActionContext::tank_swap_ghamoo;
    }

private:
    static Action* cleanse_curse(PlayerbotAI* ai)    { return new BfdCleanseCurseAction(ai); }
    static Action* dispel_magic(PlayerbotAI* ai)     { return new BfdDispelMagicAction(ai); }
    static Action* interrupt_boss(PlayerbotAI* ai)   { return new BfdInterruptBossAction(ai); }
    static Action* spread_akumai(PlayerbotAI* ai)    { return new BfdSpreadFromAkumaiAction(ai); }
    static Action* kill_sea_witch(PlayerbotAI* ai)   { return new BfdKillSeaWitchAction(ai); }
    static Action* tank_swap_ghamoo(PlayerbotAI* ai) { return new BfdTankSwapGhamooAction(ai); }
};

#endif
