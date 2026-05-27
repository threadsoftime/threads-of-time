#ifndef MOD_HARNESS_BRIDGE_OBS_PING_ADAPTER_H
#define MOD_HARNESS_BRIDGE_OBS_PING_ADAPTER_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsPing(nlohmann::json const& args);
}

#endif
