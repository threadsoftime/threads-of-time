#ifndef MOD_HARNESS_BRIDGE_BOT_QUEUE_FOR_DUNGEON_H
#define MOD_HARNESS_BRIDGE_BOT_QUEUE_FOR_DUNGEON_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult BotQueueForDungeon(nlohmann::json const& args);
}
#endif
