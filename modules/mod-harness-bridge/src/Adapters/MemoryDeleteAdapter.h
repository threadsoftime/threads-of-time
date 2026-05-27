// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_HARNESS_BRIDGE_MEMORY_DELETE_H
#define MOD_HARNESS_BRIDGE_MEMORY_DELETE_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult MemoryDelete(nlohmann::json const& args);
}

#endif
