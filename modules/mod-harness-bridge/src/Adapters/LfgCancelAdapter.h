// LfgCancelAdapter — lfg.cancel
//
// Drains pending LFG cancel intents recorded by the leave opcode hook
// (HandleLfgLeaveOpcode via RecordLfgCancel).  Called by the Rust matchmaker
// slice to receive the cancel side of the pull-based intent seam (Inc-2).
//
// C++ adapter canonical field set (must match LfgCancelArgs in tool_schemas.py):
//   args.value("max", 64)   — optional u32, default 64
//
// Response shape (consumed by harness.rs::lfg_cancel — field names are load-bearing):
//   { "cancelled": [<u64 guid_low>, ...] }

#pragma once

#include "HarnessBridgeDispatch.h"

namespace HarnessBridge::Adapters
{
    DispatchResult LfgCancel(nlohmann::json const& args);
} // namespace HarnessBridge::Adapters
