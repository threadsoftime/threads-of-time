// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_HARNESS_BRIDGE_MEMORY_RECALL_H
#define MOD_HARNESS_BRIDGE_MEMORY_RECALL_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult MemoryRecall(nlohmann::json const& args);
}

#endif
