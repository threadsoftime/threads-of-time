#ifndef MOD_HARNESS_BRIDGE_BOT_SET_GOAL_H
#define MOD_HARNESS_BRIDGE_BOT_SET_GOAL_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotSetGoal(nlohmann::json const& args);
}
#endif
