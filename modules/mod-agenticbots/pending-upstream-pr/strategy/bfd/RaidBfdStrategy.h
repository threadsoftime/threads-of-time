// SPDX-License-Identifier: GPL-2.0-or-later
//
// NOTE: This strategy is not yet wired into mod-agenticbots's startup. It depends
// on mod-playerbots exposing 4 custom-context registration methods — see
// docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-foundation.md Task 18.
// Once the upstream PR lands (or a ToT-internal patch is shipped), the
// AgenticbotsRegistration.cpp glue will register this strategy.

// RaidBfdStrategy.h
//
// ToT-specific playerbots strategy for Blackfathom Deeps as a 10-player
// raid (map 48). Mounted when PlayerbotAI::ApplyInstanceStrategies(48) fires.
//
// The 7 raid bosses are detailed in
// patches/ac-bfd-raid/overlays/.../boss_*.cpp and
// sql/bracket1-raid/world_bfd_raid_smart_scripts.sql. The strategy assumes
// those mechanics and tunes bot behavior around them.

#ifndef _PLAYERBOT_RAIDBFDSTRATEGY_H_
#define _PLAYERBOT_RAIDBFDSTRATEGY_H_

#include "Strategy.h"

class RaidBfdStrategy : public Strategy
{
public:
    RaidBfdStrategy(PlayerbotAI* ai) : Strategy(ai) {}

    std::string const getName() override { return "bfd"; }

    void InitTriggers(std::vector<TriggerNode*>& triggers) override;
    void InitMultipliers(std::vector<Multiplier*>& multipliers) override;
};

#endif
