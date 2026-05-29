// SPDX-License-Identifier: GPL-2.0-or-later
//
// NOTE: This strategy is not yet wired into mod-agenticbots's startup. It depends
// on mod-playerbots exposing 4 custom-context registration methods — see
// docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-foundation.md Task 18.
// Once the upstream PR lands (or a ToT-internal patch is shipped), the
// AgenticbotsRegistration.cpp glue will register this strategy.

#include "RaidBfdStrategy.h"

void RaidBfdStrategy::InitTriggers(std::vector<TriggerNode*>& triggers)
{
    // ----- V1: generic raid triggers that span all 7 bosses -----

    // Curse-of-Blackfathom (Gelihast) + Polymorph (Kelris). Both are curse-
    // family decursable. Dispellers (druid/mage) call their existing class
    // "remove curse" action at high priority.
    triggers.push_back(new TriggerNode(
        "bfd party member cursed",
        { NextAction("bfd cleanse curse", ACTION_DISPEL + 2) }));

    // Fear (Gelihast) + Shadow Word: Pain (Kelris). Magic dispel — priests.
    triggers.push_back(new TriggerNode(
        "bfd party member magic debuff",
        { NextAction("bfd dispel magic", ACTION_DISPEL + 2) }));

    // Boss interruptible cast (Sarevess Frost Arrow, Kelris Mind Blast).
    // Defers to the class's own interrupt action — pummel/kick/cs/earthshock.
    triggers.push_back(new TriggerNode(
        "bfd boss interruptible cast",
        { NextAction("bfd interrupt boss", ACTION_INTERRUPT + 3) }));

    // ----- V2: per-boss mechanics triggers -----

    // Aku'mai Tidal Surge (Whirlwind) — get out of melee range.
    triggers.push_back(new TriggerNode(
        "bfd akumai tidal surge",
        { NextAction("bfd spread from akumai", ACTION_EMERGENCY + 5) }));

    // Lady Sarevess Sea Witch adds spawn at 60/30% HP — focus burn.
    triggers.push_back(new TriggerNode(
        "bfd sarevess adds up",
        { NextAction("bfd kill sea witch", ACTION_RAID + 2) }));

    // Ghamoo-ra Crushing Bite (Sunder R3) stack drives a tank swap.
    triggers.push_back(new TriggerNode(
        "bfd ghamoo sunder stack",
        { NextAction("bfd tank swap ghamoo", ACTION_RAID + 4) }));
}

void RaidBfdStrategy::InitMultipliers(std::vector<Multiplier*>& /*multipliers*/)
{
    // V1: rely on existing class multipliers for dispel/interrupt priority.
    // V2 could add a BFD-specific Multiplier to boost dispel rank inside this
    // raid — for now the ACTION_DISPEL+2 priority on the trigger node is
    // enough to push past normal DPS rotations.
}
