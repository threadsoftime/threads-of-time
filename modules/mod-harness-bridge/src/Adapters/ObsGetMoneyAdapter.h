#ifndef MOD_HARNESS_BRIDGE_OBS_GET_MONEY_H
#define MOD_HARNESS_BRIDGE_OBS_GET_MONEY_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetMoney(nlohmann::json const& args);
}

#endif
