#ifndef MOD_HARNESS_BRIDGE_OBS_GET_COMBAT_LOG_H
#define MOD_HARNESS_BRIDGE_OBS_GET_COMBAT_LOG_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetCombatLog(nlohmann::json const& args);
}

#endif
