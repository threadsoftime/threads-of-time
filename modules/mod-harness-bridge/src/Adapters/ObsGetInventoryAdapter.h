#ifndef MOD_HARNESS_BRIDGE_OBS_GET_INVENTORY_H
#define MOD_HARNESS_BRIDGE_OBS_GET_INVENTORY_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetInventory(nlohmann::json const& args);
}

#endif
