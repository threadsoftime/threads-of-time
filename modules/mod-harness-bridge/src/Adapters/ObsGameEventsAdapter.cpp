// SPDX-License-Identifier: GPL-2.0-or-later
//
// ObsGameEventsAdapter — read-only dump of sGameEventMgr state +
// sHolidaysStore raw entries.
//
// Result shape:
//   server_gametime              : unix seconds (GameTime::GetGameTime().count())
//   resolve_reference_unixtime  : unix seconds (GameTime::GetStartTime().count())
//   server_tz_offset_secs        : 0 (UTC — pre-flight confirmed server TZ = UTC/no DST)
//   active_event_list            : [u16, ...]  — from GetActiveEventList()
//   events[]                     : per-event resolved schedule
//   holidays[]                   : one entry per distinct holiday id; raw sHolidaysStore dump
//
// IMPORTANT: this adapter must NOT call StartEvent / StopEvent or mutate
// _gameEvent / _activeEvents / sHolidaysStore in any way.  Pure observation.
//
// C++ adapter field contract: NONE (no args.contains/args.value calls).
// The pydantic schema ObsGameEventsArgs is `pass` (empty).

#include "Adapters/ObsGameEventsAdapter.h"

#include "GameEventMgr.h"   // sGameEventMgr, GameEventData, GameEventState
#include "GameTime.h"       // GameTime::GetGameTime, GameTime::GetStartTime
#include "DBCStores.h"      // sHolidaysStore
#include "DBCStructure.h"   // HolidaysEntry, MAX_HOLIDAY_DATES, MAX_HOLIDAY_DURATIONS

#include <set>
#include <cstdint>

namespace HarnessBridge::Adapters
{
    DispatchResult ObsGameEvents(nlohmann::json const& /*args*/)
    {
        DispatchResult res;

        // ---------- timing reference values ----------
        int64_t const server_gametime = static_cast<int64_t>(
            GameTime::GetGameTime().count());
        int64_t const resolve_ref = static_cast<int64_t>(
            GameTime::GetStartTime().count());

        // ---------- active event list ----------
        GameEventMgr::ActiveEvents const& activeSet =
            sGameEventMgr->GetActiveEventList();

        nlohmann::json active_list = nlohmann::json::array();
        for (uint16_t id : activeSet)
            active_list.push_back(static_cast<uint16_t>(id));

        // ---------- event map ----------
        GameEventMgr::GameEventDataMap const& eventMap =
            sGameEventMgr->GetEventMap();

        nlohmann::json events_arr = nlohmann::json::array();

        // Index 0 is a reserved/unused slot; valid event IDs start at 1.
        for (std::size_t i = 1; i < eventMap.size(); ++i)
        {
            GameEventData const& ev = eventMap[i];
            if (!ev.isValid())
                continue;

            uint16_t const entry = static_cast<uint16_t>(i);

            nlohmann::json ej = {
                {"entry",         entry},
                {"start",         static_cast<int64_t>(ev.Start)},
                {"end",           static_cast<int64_t>(ev.End)},
                {"occurence",     static_cast<uint32_t>(ev.Occurence)},
                {"length",        static_cast<uint32_t>(ev.Length)},
                {"holiday",       static_cast<uint32_t>(ev.HolidayId)},
                {"holiday_stage", static_cast<uint8_t>(ev.HolidayStage)},
                {"state",         static_cast<uint8_t>(ev.State)},
                {"is_active",     sGameEventMgr->IsActiveEvent(entry)},
                {"next_start",    static_cast<int64_t>(ev.NextStart)},
            };
            events_arr.push_back(std::move(ej));
        }

        // ---------- holidays (distinct holiday ids, sHolidaysStore dump) ----------
        // Collect distinct non-zero holiday ids referenced by the event map.
        std::set<uint32_t> holiday_ids_seen;
        for (GameEventData const& ev : eventMap)
        {
            if (ev.HolidayId != HOLIDAY_NONE)
                holiday_ids_seen.insert(static_cast<uint32_t>(ev.HolidayId));
        }

        nlohmann::json holidays_arr = nlohmann::json::array();
        for (uint32_t hid : holiday_ids_seen)
        {
            HolidaysEntry const* hentry = sHolidaysStore.LookupEntry(hid);
            if (!hentry)
                continue;  // holiday id referenced but not present in DBC

            // Serialize Date[MAX_HOLIDAY_DATES] (26 u32 values).
            nlohmann::json date_arr = nlohmann::json::array();
            for (int d = 0; d < MAX_HOLIDAY_DATES; ++d)
                date_arr.push_back(hentry->Date[d]);

            // Serialize Duration[MAX_HOLIDAY_DURATIONS] (10 u32 values, hours).
            nlohmann::json dur_arr = nlohmann::json::array();
            for (int d = 0; d < MAX_HOLIDAY_DURATIONS; ++d)
                dur_arr.push_back(hentry->Duration[d]);

            // Note: CalendarFlags[MAX_HOLIDAY_FLAGS] sits between Looping and
            // CalendarFilterType in the struct layout — we skip it as the Rust
            // slice does not consume it.  Only the fields below are serialized.
            nlohmann::json hj = {
                {"holiday_id",          hid},
                {"date",                std::move(date_arr)},
                {"duration",            std::move(dur_arr)},
                {"calendar_filter_type", static_cast<int32_t>(hentry->CalendarFilterType)},
                {"looping",             hentry->Looping},
                {"region",              hentry->Region},
            };
            holidays_arr.push_back(std::move(hj));
        }

        // ---------- assemble result ----------
        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"server_gametime",             server_gametime},
            {"resolve_reference_unixtime",  resolve_ref},
            {"server_tz_offset_secs",       0},
            {"active_event_list",           std::move(active_list)},
            {"events",                      std::move(events_arr)},
            {"holidays",                    std::move(holidays_arr)},
        };
        return res;
    }
}
