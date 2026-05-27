#ifndef MOD_HARNESS_BRIDGE_OBS_GET_STATE_H
#define MOD_HARNESS_BRIDGE_OBS_GET_STATE_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetState(nlohmann::json const& args);
}

#endif
