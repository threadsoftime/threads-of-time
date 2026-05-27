#ifndef MOD_HARNESS_BRIDGE_OBS_GET_XP_H
#define MOD_HARNESS_BRIDGE_OBS_GET_XP_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetXp(nlohmann::json const& args);
}

#endif
