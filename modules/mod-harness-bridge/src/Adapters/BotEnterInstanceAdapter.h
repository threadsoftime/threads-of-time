#ifndef MOD_HARNESS_BRIDGE_BOT_ENTER_INSTANCE_H
#define MOD_HARNESS_BRIDGE_BOT_ENTER_INSTANCE_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotEnterInstance(nlohmann::json const& args);
}
#endif
