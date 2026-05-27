#ifndef MOD_HARNESS_BRIDGE_BOT_SET_STRATEGY_H
#define MOD_HARNESS_BRIDGE_BOT_SET_STRATEGY_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotSetStrategy(nlohmann::json const& args);
}
#endif
