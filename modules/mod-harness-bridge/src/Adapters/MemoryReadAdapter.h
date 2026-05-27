// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_HARNESS_BRIDGE_MEMORY_READ_H
#define MOD_HARNESS_BRIDGE_MEMORY_READ_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult MemoryRead(nlohmann::json const& args);
}

#endif
