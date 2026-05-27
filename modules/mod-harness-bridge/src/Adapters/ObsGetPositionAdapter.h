#ifndef MOD_HARNESS_BRIDGE_OBS_GET_POSITION_H
#define MOD_HARNESS_BRIDGE_OBS_GET_POSITION_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetPosition(nlohmann::json const& args);
}

#endif
