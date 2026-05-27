#ifndef MOD_HARNESS_BRIDGE_CONFIG_H
#define MOD_HARNESS_BRIDGE_CONFIG_H

#include "Common.h"
#include <string>

namespace HarnessBridge
{
    struct Config
    {
        bool        Enabled              = true;
        std::string BindAddress          = "127.0.0.1";
        uint16      Port                 = 8091;
        uint32      MaxDispatchPerTick   = 16;
        uint32      QueueCap             = 256;
        uint32      RequestTimeoutMs     = 2000;
        std::string AuditPath            = "/azerothcore/env/dist/logs/harness_bridge.jsonl";
        uint32      LoggerLevel          = 3;
    };

    Config const& GetConfig();
    void          LoadConfig();
}

#endif // MOD_HARNESS_BRIDGE_CONFIG_H
