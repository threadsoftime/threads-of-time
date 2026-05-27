#ifndef MOD_HARNESS_BRIDGE_OBS_GET_GROUP_H
#define MOD_HARNESS_BRIDGE_OBS_GET_GROUP_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetGroup(nlohmann::json const& args);
}

#endif
