// SPDX-License-Identifier: GPL-2.0-or-later
//
// NOTE: This context is not yet wired into mod-agenticbots's startup. It depends
// on mod-playerbots exposing 4 custom-context registration methods — see
// docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-foundation.md Task 18.
// Once the upstream PR lands (or a ToT-internal patch is shipped), the
// AgenticbotsRegistration.cpp glue will register this context.

#ifndef _PLAYERBOT_RAIDBFDTRIGGERCONTEXT_H_
#define _PLAYERBOT_RAIDBFDTRIGGERCONTEXT_H_

#include "AiObjectContext.h"
#include "NamedObjectContext.h"
#include "RaidBfdTriggers.h"

class RaidBfdTriggerContext : public NamedObjectContext<Trigger>
{
public:
    RaidBfdTriggerContext()
    {
        creators["bfd party member cursed"]      = &RaidBfdTriggerContext::cursed;
        creators["bfd party member magic debuff"] = &RaidBfdTriggerContext::magic_debuff;
        creators["bfd boss interruptible cast"]  = &RaidBfdTriggerContext::interruptible;
        creators["bfd akumai tidal surge"]       = &RaidBfdTriggerContext::akumai_surge;
        creators["bfd sarevess adds up"]         = &RaidBfdTriggerContext::sarevess_adds;
        creators["bfd ghamoo sunder stack"]      = &RaidBfdTriggerContext::ghamoo_sunder;
    }

private:
    static Trigger* cursed(PlayerbotAI* ai)        { return new BfdPartyMemberCursedTrigger(ai); }
    static Trigger* magic_debuff(PlayerbotAI* ai)  { return new BfdPartyMemberMagicDebuffTrigger(ai); }
    static Trigger* interruptible(PlayerbotAI* ai) { return new BfdBossInterruptibleCastTrigger(ai); }
    static Trigger* akumai_surge(PlayerbotAI* ai)  { return new BfdAkumaiTidalSurgeTrigger(ai); }
    static Trigger* sarevess_adds(PlayerbotAI* ai) { return new BfdSarevessAddsUpTrigger(ai); }
    static Trigger* ghamoo_sunder(PlayerbotAI* ai) { return new BfdGhamooSunderStackTrigger(ai); }
};

#endif
