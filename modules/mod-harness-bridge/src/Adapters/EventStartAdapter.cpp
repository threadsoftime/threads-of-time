// SPDX-License-Identifier: GPL-2.0-or-later
//
// EventStartAdapter — activate a game event via GameEventMgr::StartEvent.
//
// Args: { "event_id": <uint16> }
//
// Validates:
//   1. event_id present and numeric
//   2. 1 <= event_id < _gameEvent.size()   (index in bounds)
//   3. _gameEvent[event_id].isValid()       (Length>0 or State>NORMAL — not a tombstone)
//
// On success, calls sGameEventMgr->StartEvent(event_id, false).
//   overwrite=false: does NOT rebase the Start timestamp — the slice drives transitions
//   without claiming ownership of the date metadata.
//
// Result: { "started": true, "event_id": <u16>, "is_active_now": <bool> }
//
// NOTE: this adapter is main-thread-only (runs inside OnTickDrain, which is called
//       from WorldScript::OnUpdate).  Thread safety: same rules as the rest of the
//       adapter table — no additional locks required.

#include "Adapters/EventStartAdapter.h"

#include "GameEventMgr.h"   // sGameEventMgr, GetEventMap
#include "Log.h"

namespace HarnessBridge::Adapters
{
    DispatchResult EventStart(nlohmann::json const& args)
    {
        DispatchResult res;

        // ---- validate args ----
        if (!args.contains("event_id") || !args["event_id"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "event.start: event_id (integer) required";
            return res;
        }

        auto const raw_id = args["event_id"].get<int64_t>();

        // Bounds check — event IDs are uint16, valid range 1..size()-1.
        GameEventMgr::GameEventDataMap const& eventMap = sGameEventMgr->GetEventMap();
        if (raw_id < 1 || static_cast<std::size_t>(raw_id) >= eventMap.size())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "event.start: event_id out of range [1, " +
                                std::to_string(eventMap.size() - 1) + "]";
            return res;
        }

        auto const event_id = static_cast<uint16>(raw_id);

        // isValid() — rejects tombstone/placeholder entries.
        if (!eventMap[event_id].isValid())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "event.start: event_id " + std::to_string(event_id) +
                                " is not a valid event (Length==0 and State==NORMAL)";
            return res;
        }

        // ---- execute ----
        // overwrite=false: preserve the original Start/End dates; the slice drives
        // transitions without rebasing the date metadata.
        sGameEventMgr->StartEvent(event_id, false);

        bool const is_active_now = sGameEventMgr->IsActiveEvent(event_id);

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"started",      true},
            {"event_id",     event_id},
            {"is_active_now", is_active_now},
        };
        return res;
    }
}
