#ifndef MOD_HARNESS_BRIDGE_BOT_STOP_H
#define MOD_HARNESS_BRIDGE_BOT_STOP_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotStop(nlohmann::json const& args);
}
#endif
