// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_HARNESS_BRIDGE_OBS_LIST_BOT_POPULATION_H
#define MOD_HARNESS_BRIDGE_OBS_LIST_BOT_POPULATION_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsListBotPopulation(nlohmann::json const& args);
}

#endif
