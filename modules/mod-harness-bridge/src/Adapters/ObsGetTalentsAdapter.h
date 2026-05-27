#ifndef MOD_HARNESS_BRIDGE_OBS_GET_TALENTS_H
#define MOD_HARNESS_BRIDGE_OBS_GET_TALENTS_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult ObsGetTalents(nlohmann::json const& args);
}
#endif
