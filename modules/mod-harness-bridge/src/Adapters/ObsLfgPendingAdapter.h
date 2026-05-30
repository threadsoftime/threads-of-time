// ObsLfgPendingAdapter — obs.lfg_pending
//
// Drains pending real-player (and future bot) LFG join intents recorded
// by the veto hook.  Called by the Rust matchmaker slice to receive the
// real-player side of the pull-based intent seam (Stage 3, Inc-1).
//
// C++ adapter canonical field set (must match ObsLfgPendingArgs in tool_schemas.py):
//   args.value("max", 64)   — optional u32, default 64
//
// Response shape (consumed by harness.rs::lfg_pending — field names are load-bearing):
//   { "pending": [ { "guid": <u64>, "team_id": <u8>, "roles": <u32>,
//                    "dungeon_ids": [<u32>...], "comment": "<str>",
//                    "is_bot": <bool> } ] }

#pragma once

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult ObsLfgPending(nlohmann::json const& args);
} // namespace HarnessBridge::Adapters
