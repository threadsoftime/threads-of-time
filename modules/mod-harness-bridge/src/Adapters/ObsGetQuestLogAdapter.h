// modules/mod-harness-bridge/src/Adapters/ObsGetQuestLogAdapter.h
#ifndef MOD_HARNESS_BRIDGE_OBS_GET_QUEST_LOG_H
#define MOD_HARNESS_BRIDGE_OBS_GET_QUEST_LOG_H

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGetQuestLog(nlohmann::json const& args);
}

#endif
