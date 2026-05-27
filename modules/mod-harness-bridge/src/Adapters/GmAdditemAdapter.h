#ifndef MOD_HARNESS_BRIDGE_GM_ADDITEM_H
#define MOD_HARNESS_BRIDGE_GM_ADDITEM_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult GmAdditem(nlohmann::json const& args);
}
#endif
