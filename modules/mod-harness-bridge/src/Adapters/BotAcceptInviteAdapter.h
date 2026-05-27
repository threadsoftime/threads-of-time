#ifndef MOD_HARNESS_BRIDGE_BOT_ACCEPT_INVITE_H
#define MOD_HARNESS_BRIDGE_BOT_ACCEPT_INVITE_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotAcceptInvite(nlohmann::json const& args);
}
#endif
