#include "HarnessBridgeConfig.h"

#include "Config.h"
#include "Log.h"

namespace HarnessBridge
{
    namespace
    {
        Config gConfig;
    }

    Config const& GetConfig() { return gConfig; }

    void LoadConfig()
    {
        gConfig.Enabled            = sConfigMgr->GetOption<bool>       ("HarnessBridge.Enable",            true);
        gConfig.BindAddress        = sConfigMgr->GetOption<std::string>("HarnessBridge.BindAddress",       "127.0.0.1");
        gConfig.Port               = sConfigMgr->GetOption<uint16>     ("HarnessBridge.Port",              8091);
        gConfig.MaxDispatchPerTick = sConfigMgr->GetOption<uint32>     ("HarnessBridge.MaxDispatchPerTick",16);
        gConfig.QueueCap           = sConfigMgr->GetOption<uint32>     ("HarnessBridge.QueueCap",          256);
        gConfig.RequestTimeoutMs   = sConfigMgr->GetOption<uint32>     ("HarnessBridge.RequestTimeoutMs",  2000);
        gConfig.AuditPath          = sConfigMgr->GetOption<std::string>("HarnessBridge.AuditPath",         "/azerothcore/env/dist/logs/harness_bridge.jsonl");
        gConfig.LoggerLevel        = sConfigMgr->GetOption<uint32>     ("HarnessBridge.LoggerLevel",       3);

        LOG_INFO("server.loading",
                 "[mod-harness-bridge] Config: Enable={} Bind={}:{} MaxPerTick={} QueueCap={} TimeoutMs={}",
                 gConfig.Enabled ? 1 : 0,
                 gConfig.BindAddress, gConfig.Port,
                 gConfig.MaxDispatchPerTick, gConfig.QueueCap, gConfig.RequestTimeoutMs);
    }
}
