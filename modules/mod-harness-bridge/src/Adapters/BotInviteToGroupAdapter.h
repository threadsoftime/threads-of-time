#ifndef MOD_HARNESS_BRIDGE_BOT_INVITE_TO_GROUP_H
#define MOD_HARNESS_BRIDGE_BOT_INVITE_TO_GROUP_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotInviteToGroup(nlohmann::json const& args);
}
#endif
