#ifndef MOD_HARNESS_BRIDGE_OBS_GET_RPG_STATUS_H
#define MOD_HARNESS_BRIDGE_OBS_GET_RPG_STATUS_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetRpgStatus(nlohmann::json const& args);
}

#endif
