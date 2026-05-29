// SPDX-License-Identifier: GPL-2.0-or-later
//
// NOTE: These triggers are not yet wired into mod-agenticbots's startup. They depend
// on mod-playerbots exposing 4 custom-context registration methods — see
// docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-foundation.md Task 18.
// Once the upstream PR lands (or a ToT-internal patch is shipped), the
// AgenticbotsRegistration.cpp glue will register these triggers via RaidBfdTriggerContext.

// RaidBfdTriggers.h
#ifndef _PLAYERBOT_RAIDBFDTRIGGERS_H_
#define _PLAYERBOT_RAIDBFDTRIGGERS_H_

#include "PlayerbotAI.h"
#include "Trigger.h"

// -------- V1 generic raid triggers --------

class BfdPartyMemberCursedTrigger : public Trigger
{
public:
    BfdPartyMemberCursedTrigger(PlayerbotAI* botAI);
    bool IsActive() override;
};

class BfdPartyMemberMagicDebuffTrigger : public Trigger
{
public:
    BfdPartyMemberMagicDebuffTrigger(PlayerbotAI* botAI);
    bool IsActive() override;
};

class BfdBossInterruptibleCastTrigger : public Trigger
{
public:
    BfdBossInterruptibleCastTrigger(PlayerbotAI* botAI);
    bool IsActive() override;
};

// -------- V2 per-boss triggers --------

class BfdAkumaiTidalSurgeTrigger : public Trigger
{
public:
    BfdAkumaiTidalSurgeTrigger(PlayerbotAI* botAI);
    bool IsActive() override;
};

class BfdSarevessAddsUpTrigger : public Trigger
{
public:
    BfdSarevessAddsUpTrigger(PlayerbotAI* botAI);
    bool IsActive() override;
};

class BfdGhamooSunderStackTrigger : public Trigger
{
public:
    BfdGhamooSunderStackTrigger(PlayerbotAI* botAI);
    bool IsActive() override;
};

#endif
