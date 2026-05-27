#ifndef MOD_HARNESS_BRIDGE_OBS_GET_AURAS_H
#define MOD_HARNESS_BRIDGE_OBS_GET_AURAS_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetAuras(nlohmann::json const& args);
}

#endif
