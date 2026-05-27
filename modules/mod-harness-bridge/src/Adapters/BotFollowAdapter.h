#ifndef MOD_HARNESS_BRIDGE_BOT_FOLLOW_H
#define MOD_HARNESS_BRIDGE_BOT_FOLLOW_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotFollow(nlohmann::json const& args);
}
#endif
