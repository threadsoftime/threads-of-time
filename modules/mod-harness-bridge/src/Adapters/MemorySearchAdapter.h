// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_HARNESS_BRIDGE_MEMORY_SEARCH_H
#define MOD_HARNESS_BRIDGE_MEMORY_SEARCH_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult MemorySearch(nlohmann::json const& args);
}

#endif
