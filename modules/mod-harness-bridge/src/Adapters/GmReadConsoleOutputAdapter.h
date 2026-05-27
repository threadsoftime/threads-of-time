#ifndef MOD_HARNESS_BRIDGE_GM_READ_CONSOLE_OUTPUT_ADAPTER_H
#define MOD_HARNESS_BRIDGE_GM_READ_CONSOLE_OUTPUT_ADAPTER_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult GmReadConsoleOutput(nlohmann::json const& args);
}

#endif
