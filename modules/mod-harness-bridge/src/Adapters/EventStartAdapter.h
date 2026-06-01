// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_HARNESS_BRIDGE_EVENT_START_ADAPTER_H
#define MOD_HARNESS_BRIDGE_EVENT_START_ADAPTER_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult EventStart(nlohmann::json const& args);
}

#endif
