// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_HARNESS_BRIDGE_OBS_GAME_EVENTS_ADAPTER_H
#define MOD_HARNESS_BRIDGE_OBS_GAME_EVENTS_ADAPTER_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGameEvents(nlohmann::json const& args);
}

#endif
