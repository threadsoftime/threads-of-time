// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_HARNESS_BRIDGE_OBS_LIST_PLAYERS_H
#define MOD_HARNESS_BRIDGE_OBS_LIST_PLAYERS_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsListPlayers(nlohmann::json const& args);
}

#endif
