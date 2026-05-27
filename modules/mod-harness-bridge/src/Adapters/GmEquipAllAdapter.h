#ifndef MOD_HARNESS_BRIDGE_GM_EQUIP_ALL_H
#define MOD_HARNESS_BRIDGE_GM_EQUIP_ALL_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult GmEquipAll(nlohmann::json const& args);
}
#endif
