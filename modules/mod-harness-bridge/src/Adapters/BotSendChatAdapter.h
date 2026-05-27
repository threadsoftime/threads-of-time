#ifndef MOD_HARNESS_BRIDGE_BOT_SEND_CHAT_H
#define MOD_HARNESS_BRIDGE_BOT_SEND_CHAT_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotSendChat(nlohmann::json const& args);
}
#endif
