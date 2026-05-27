#ifndef MOD_HARNESS_BRIDGE_GM_SET_LEVEL_H
#define MOD_HARNESS_BRIDGE_GM_SET_LEVEL_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult GmSetLevel(nlohmann::json const& args);
}
#endif
