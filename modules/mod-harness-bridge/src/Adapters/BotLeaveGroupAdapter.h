#ifndef MOD_HARNESS_BRIDGE_BOT_LEAVE_GROUP_H
#define MOD_HARNESS_BRIDGE_BOT_LEAVE_GROUP_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotLeaveGroup(nlohmann::json const& args);
}
#endif
