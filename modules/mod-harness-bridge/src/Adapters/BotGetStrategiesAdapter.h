#ifndef MOD_HARNESS_BRIDGE_BOT_GET_STRATEGIES_H
#define MOD_HARNESS_BRIDGE_BOT_GET_STRATEGIES_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotGetStrategies(nlohmann::json const& args);
}
#endif
