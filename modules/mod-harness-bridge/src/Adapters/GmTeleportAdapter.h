#ifndef MOD_HARNESS_BRIDGE_GM_TELEPORT_H
#define MOD_HARNESS_BRIDGE_GM_TELEPORT_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult GmTeleport(nlohmann::json const& args);
}
#endif
