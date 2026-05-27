// SPDX-License-Identifier: GPL-2.0-or-later
//
// NOTE: These actions are not yet wired into mod-agenticbots's startup. They depend
// on mod-playerbots exposing 4 custom-context registration methods — see
// docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-foundation.md Task 18.
// Once the upstream PR lands (or a Heimdal-internal patch is shipped), the
// AgenticbotsRegistration.cpp glue will register these actions via RaidBfdActionContext.

// RaidBfdActions.h
#ifndef _PLAYERBOT_RAIDBFDACTIONS_H_
#define _PLAYERBOT_RAIDBFDACTIONS_H_

#include "Action.h"
#include "AttackAction.h"
#include "GenericSpellActions.h"
#include "MovementActions.h"

class PlayerbotAI;

// V1: delegate to the class's existing cleanse/dispel/interrupt actions by
// invoking the named action through the bot's action context. Each of these
// is a thin wrapper that fires the appropriate class-defined action when
// available; if the class doesn't have one (e.g. a rogue can't decurse), the
// action returns false and the engine moves on.

class BfdCleanseCurseAction : public Action
{
public:
    BfdCleanseCurseAction(PlayerbotAI* botAI) : Action(botAI, "bfd cleanse curse") {}
    bool Execute(Event event) override;
};

class BfdDispelMagicAction : public Action
{
public:
    BfdDispelMagicAction(PlayerbotAI* botAI) : Action(botAI, "bfd dispel magic") {}
    bool Execute(Event event) override;
};

class BfdInterruptBossAction : public Action
{
public:
    BfdInterruptBossAction(PlayerbotAI* botAI) : Action(botAI, "bfd interrupt boss") {}
    bool Execute(Event event) override;
};

// V2: BFD-specific behavior

class BfdSpreadFromAkumaiAction : public MovementAction
{
public:
    BfdSpreadFromAkumaiAction(PlayerbotAI* botAI) : MovementAction(botAI, "bfd spread from akumai") {}
    bool Execute(Event event) override;
};

class BfdKillSeaWitchAction : public AttackAction
{
public:
    BfdKillSeaWitchAction(PlayerbotAI* botAI) : AttackAction(botAI, "bfd kill sea witch") {}
    bool Execute(Event event) override;
};

class BfdTankSwapGhamooAction : public Action
{
public:
    BfdTankSwapGhamooAction(PlayerbotAI* botAI) : Action(botAI, "bfd tank swap ghamoo") {}
    bool Execute(Event event) override;
};

#endif
