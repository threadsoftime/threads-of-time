#ifndef MOD_HARNESS_BRIDGE_GM_STRIP_GEAR_ADAPTER_H
#define MOD_HARNESS_BRIDGE_GM_STRIP_GEAR_ADAPTER_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult GmStripGear(nlohmann::json const& args);
}

#endif
