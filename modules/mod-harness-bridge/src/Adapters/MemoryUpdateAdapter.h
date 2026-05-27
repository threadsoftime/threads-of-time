// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_HARNESS_BRIDGE_MEMORY_UPDATE_H
#define MOD_HARNESS_BRIDGE_MEMORY_UPDATE_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult MemoryUpdate(nlohmann::json const& args);
}

#endif
