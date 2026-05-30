#include "HarnessBridge.h"
#include "HarnessBridgeConfig.h"
#include "HarnessBridgeDispatch.h"
#include "HarnessBridgeServer.h"
#include "ConsoleCaptureStore.h"
#include "CombatLogBuffer.h"
#include "Handlers/DispatchHandler.h"
#include "Handlers/HealthHandler.h"
#include "Log.h"
#include "ScriptMgr.h"

#include <chrono>
#include <memory>

namespace
{
    std::unique_ptr<HarnessBridge::Server> gServer;

    class HarnessBridgeWorldScript : public WorldScript
    {
    public:
        HarnessBridgeWorldScript() : WorldScript("HarnessBridgeWorldScript") { }

        void OnAfterConfigLoad(bool /*reload*/) override
        {
            HarnessBridge::LoadConfig();
        }

        void OnStartup() override
        {
            auto const& cfg = HarnessBridge::GetConfig();
            if (!cfg.Enabled)
            {
                LOG_INFO("server.loading", "[mod-harness-bridge] disabled by config");
                return;
            }

            gServer = std::make_unique<HarnessBridge::Server>();
            HarnessBridge::RegisterHealthHandler(gServer->Raw());
            HarnessBridge::RegisterDispatchHandler(gServer->Raw());

            if (!gServer->Start(cfg.BindAddress, cfg.Port))
            {
                LOG_ERROR("server.loading",
                          "[mod-harness-bridge] failed to start server on {}:{}",
                          cfg.BindAddress, cfg.Port);
                gServer.reset();
                return;
            }
            LOG_INFO("server.loading",
                     "[mod-harness-bridge] ready (handlers: /health, /dispatch; 19 V1.3 tools registered) on {}:{}",
                     cfg.BindAddress, cfg.Port);
        }

        void OnUpdate(uint32 /*diff*/) override
        {
            // Called every worldserver tick. Bounded by MaxDispatchPerTick.
            HarnessBridge::OnTickDrain();
            HarnessBridge::ConsoleCaptureStore::Instance().EvictExpired(
                std::chrono::steady_clock::now());
        }

        void OnShutdown() override
        {
            if (gServer)
            {
                LOG_INFO("server.loading", "[mod-harness-bridge] shutting down server");
                gServer->Stop();
                gServer.reset();
            }
        }
    };

    class HarnessBridgeUnitScript : public UnitScript
    {
    public:
        HarnessBridgeUnitScript() : UnitScript("HarnessBridgeUnitScript") { }

        // Called when DealDamage runs. We snapshot the event into our ring
        // buffer with monotonic ms timestamp.
        void OnDamage(Unit* attacker, Unit* victim, uint32& damage) override
        {
            if (!attacker || !victim)
                return;
            HarnessBridge::CombatEvent ev;
            ev.ts_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::steady_clock::now().time_since_epoch()).count();
            ev.spell_id = 0;
            ev.source_guid = attacker->GetGUID().GetRawValue();
            ev.target_guid = victim->GetGUID().GetRawValue();
            ev.amount = static_cast<int32>(damage);
            ev.kind = "damage";
            ev.school = "unknown";
            HarnessBridge::Buffer().Append(std::move(ev));
        }

        void OnHeal(Unit* healer, Unit* recipient, uint32& gain) override
        {
            if (!healer || !recipient)
                return;
            HarnessBridge::CombatEvent ev;
            ev.ts_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::steady_clock::now().time_since_epoch()).count();
            ev.source_guid = healer->GetGUID().GetRawValue();
            ev.target_guid = recipient->GetGUID().GetRawValue();
            ev.amount = static_cast<int32>(gain);
            ev.kind = "heal";
            HarnessBridge::Buffer().Append(std::move(ev));
        }
    };
}

void AddHarnessBridgeScripts()
{
    new HarnessBridgeWorldScript();
    new HarnessBridgeUnitScript();
    // Stage 3 Inc-1: veto hook intercepts real-player LFG joins and records
    // intent into LfgIntentStore for the Rust matchmaker slice to drain.
    AddLfgVetoScript();
}
