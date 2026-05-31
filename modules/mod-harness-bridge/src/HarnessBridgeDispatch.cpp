#include "HarnessBridgeDispatch.h"

#include "HarnessBridgeConfig.h"
#include "Log.h"

#include <chrono>
#include <exception>

namespace HarnessBridge
{
    bool DispatchQueue::TryPush(std::unique_ptr<PendingWork> work)
    {
        std::lock_guard<std::mutex> lk(_mu);
        if (_q.size() >= _cap)
            return false;
        _q.push(std::move(work));
        return true;
    }

    std::size_t DispatchQueue::DrainUpTo(std::size_t max,
                                         std::vector<std::unique_ptr<PendingWork>>& out)
    {
        std::lock_guard<std::mutex> lk(_mu);
        std::size_t taken = 0;
        while (taken < max && !_q.empty())
        {
            out.push_back(std::move(_q.front()));
            _q.pop();
            ++taken;
        }
        return taken;
    }

    std::size_t DispatchQueue::Size() const
    {
        std::lock_guard<std::mutex> lk(_mu);
        return _q.size();
    }

    namespace
    {
        DispatchQueue& InstanceForConfig()
        {
            static DispatchQueue q(GetConfig().QueueCap);
            return q;
        }
    }

    DispatchQueue& Queue()
    {
        return InstanceForConfig();
    }

    // Forward declaration: real implementation + lookup table live at
    // the bottom of this file (after the namespace closes and reopens).
    DispatchResult RunAdapter(std::string const& tool_name,
                              nlohmann::json const& args_json);

    void OnTickDrain()
    {
        auto const& cfg = GetConfig();
        if (!cfg.Enabled)
            return;

        thread_local std::vector<std::unique_ptr<PendingWork>> batch;
        batch.clear();
        Queue().DrainUpTo(cfg.MaxDispatchPerTick, batch);

        for (auto& work : batch)
        {
            // Capture tick_wait_ms BEFORE running RunAdapter — RunAdapter
            // returns a fresh DispatchResult that would otherwise clobber
            // any field we set on `res` upfront. Both timing fields are
            // written after the try/catch so the success and failure
            // paths agree.
            auto const now = std::chrono::steady_clock::now();
            uint64 const tick_wait_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                now - work->enqueued_at).count();

            DispatchResult res;
            auto exec_start = std::chrono::steady_clock::now();
            try
            {
                res = RunAdapter(work->tool_name, work->args_json);
            }
            catch (std::exception const& e)
            {
                res.outcome = DispatchResult::Outcome::ExecutorThrew;
                res.error_message = e.what();
            }
            res.tick_wait_ms = tick_wait_ms;
            res.executor_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::steady_clock::now() - exec_start).count();

            try
            {
                work->result_promise.set_value(std::move(res));
            }
            catch (std::future_error const&)
            {
                // Handler thread already abandoned the future (timeout). Drop result.
            }
        }
    }
}

#include "Adapters/ObsPingAdapter.h"
#include "Adapters/ObsGetStateAdapter.h"
#include "Adapters/ObsGetAurasAdapter.h"
#include "Adapters/ObsGetInventoryAdapter.h"
#include "Adapters/ObsGetCombatLogAdapter.h"
#include "Adapters/GmAdditemAdapter.h"
#include "Adapters/GmEquipAllAdapter.h"
#include "Adapters/GmTeleportAdapter.h"
#include "Adapters/GmSetLevelAdapter.h"
#include "Adapters/GmRunConsoleAdapter.h"
#include "Adapters/GmReadConsoleOutputAdapter.h"
#include "Adapters/GmStripGearAdapter.h"
#include "Adapters/BotSetGoalAdapter.h"
#include "Adapters/ObsGetQuestLogAdapter.h"
#include "Adapters/ObsGetXpAdapter.h"
#include "Adapters/ObsGetRpgStatusAdapter.h"
#include "Adapters/ObsGetMoneyAdapter.h"
#include "Adapters/ObsGetPositionAdapter.h"
#include "Adapters/ObsGetGroupAdapter.h"
#include "Adapters/ObsListBotPopulationAdapter.h"
#include "Adapters/ObsListPlayersAdapter.h"
#include "Adapters/BotSetStrategyAdapter.h"
#include "Adapters/BotGetStrategiesAdapter.h"
#include "Adapters/BotSendChatAdapter.h"
#include "Adapters/BotFollowAdapter.h"
#include "Adapters/BotStopAdapter.h"
#include "Adapters/ObsGetTalentsAdapter.h"
#include "Adapters/BotInviteToGroupAdapter.h"
#include "Adapters/BotAcceptInviteAdapter.h"
#include "Adapters/BotLeaveGroupAdapter.h"
#include "Adapters/BotSetRoleAdapter.h"
#include "Adapters/BotQueueForDungeonAdapter.h"
#include "Adapters/BotEnterInstanceAdapter.h"
#include "Adapters/MemoryWriteAdapter.h"
#include "Adapters/MemoryReadAdapter.h"
#include "Adapters/MemoryRecallAdapter.h"
#include "Adapters/MemorySearchAdapter.h"
#include "Adapters/MemoryListAdapter.h"
#include "Adapters/MemoryUpdateAdapter.h"
#include "Adapters/MemoryDeleteAdapter.h"
#include "Adapters/ObsGameEventsAdapter.h"
#include "Adapters/EventStartAdapter.h"
#include "Adapters/EventStopAdapter.h"

#include <unordered_map>
#include <functional>

namespace HarnessBridge
{
    using AdapterFn = std::function<DispatchResult(nlohmann::json const&)>;

    static std::unordered_map<std::string, AdapterFn> const& AdapterTable()
    {
        // V1.5 ships 31 tools (= V1.4's 25 + 6 grouping primitives:
        // bot.invite_to_group, bot.accept_invite, bot.leave_group,
        // bot.set_role, bot.queue_for_dungeon, bot.enter_instance).
        // V1.5+ memory.* (Tasks 24-30): 7 adapters calling tot/memory sidecar
        // via HARNESS_MEMORY_URL (default http://localhost:8090).
        // Plan-3 subset-gating (Tasks 4-5): +2 world-snapshot tools:
        // obs.list_players, obs.list_bot_population.
        // GES Inc-1 (Task 9): +1 obs.game_events (resolved schedule +
        // active set + sHolidaysStore dump — read-only).
        // GES Inc-2 (Task A): +2 mutating event primitives:
        // event.start, event.stop (slice-driven transitions).
        // Total C++ adapters: 43. Daemon-direct obs.query_db brings the
        // total exposed surface to 44 tools across HTTP /v1/* and MCP.
        static std::unordered_map<std::string, AdapterFn> const table = {
            {"obs.ping",                  &Adapters::ObsPing},
            {"obs.get_state",             &Adapters::ObsGetState},
            {"obs.get_auras",             &Adapters::ObsGetAuras},
            {"obs.get_inventory",         &Adapters::ObsGetInventory},
            {"obs.get_combat_log",        &Adapters::ObsGetCombatLog},
            {"obs.get_quest_log",         &Adapters::ObsGetQuestLog},
            {"obs.get_xp",                &Adapters::ObsGetXp},
            {"obs.get_rpg_status",        &Adapters::ObsGetRpgStatus},
            {"obs.get_money",             &Adapters::ObsGetMoney},
            {"obs.get_position",          &Adapters::ObsGetPosition},
            {"obs.get_group",             &Adapters::ObsGetGroup},
            {"obs.list_bot_population",   &Adapters::ObsListBotPopulation},
            {"obs.list_players",          &Adapters::ObsListPlayers},
            {"obs.game_events",           &Adapters::ObsGameEvents},
            {"event.start",               &Adapters::EventStart},
            {"event.stop",                &Adapters::EventStop},
            {"gm.additem",                &Adapters::GmAdditem},
            {"gm.equip_all",              &Adapters::GmEquipAll},
            {"gm.teleport",               &Adapters::GmTeleport},
            {"gm.set_level",              &Adapters::GmSetLevel},
            {"gm.run_console",            &Adapters::GmRunConsole},
            {"gm.read_console_output",    &Adapters::GmReadConsoleOutput},
            {"gm.strip_gear",             &Adapters::GmStripGear},
            {"bot.set_goal",              &Adapters::BotSetGoal},
            {"bot.set_strategy",          &Adapters::BotSetStrategy},
            {"bot.get_strategies",        &Adapters::BotGetStrategies},
            {"bot.send_chat",             &Adapters::BotSendChat},
            {"bot.follow",                &Adapters::BotFollow},
            {"bot.stop",                  &Adapters::BotStop},
            {"obs.get_talents",           &Adapters::ObsGetTalents},
            {"bot.invite_to_group",   &Adapters::BotInviteToGroup},
            {"bot.accept_invite",     &Adapters::BotAcceptInvite},
            {"bot.leave_group",       &Adapters::BotLeaveGroup},
            {"bot.set_role",          &Adapters::BotSetRole},
            {"bot.queue_for_dungeon", &Adapters::BotQueueForDungeon},
            {"bot.enter_instance",    &Adapters::BotEnterInstance},
            {"memory.write",          &Adapters::MemoryWrite},
            {"memory.read",           &Adapters::MemoryRead},
            {"memory.recall",         &Adapters::MemoryRecall},
            {"memory.search",         &Adapters::MemorySearch},
            {"memory.list",           &Adapters::MemoryList},
            {"memory.update",         &Adapters::MemoryUpdate},
            {"memory.delete",         &Adapters::MemoryDelete},
        };
        return table;
    }

    DispatchResult RunAdapter(std::string const& tool_name,
                              nlohmann::json const& args_json)
    {
        auto const& tbl = AdapterTable();
        auto it = tbl.find(tool_name);
        if (it == tbl.end())
        {
            DispatchResult res;
            res.outcome = DispatchResult::Outcome::UnknownTool;
            res.error_message = "unknown tool: " + tool_name;
            return res;
        }
        return it->second(args_json);
    }
}
