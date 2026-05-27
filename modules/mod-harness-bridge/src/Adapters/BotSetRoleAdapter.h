#ifndef MOD_HARNESS_BRIDGE_BOT_SET_ROLE_H
#define MOD_HARNESS_BRIDGE_BOT_SET_ROLE_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotSetRole(nlohmann::json const& args);
}
#endif
