#ifndef MOD_HARNESS_BRIDGE_GM_RUN_CONSOLE_H
#define MOD_HARNESS_BRIDGE_GM_RUN_CONSOLE_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult GmRunConsole(nlohmann::json const& args);
}
#endif
