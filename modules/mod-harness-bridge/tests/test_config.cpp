#define DOCTEST_CONFIG_IMPLEMENT_WITH_MAIN
#include <doctest/doctest.h>

#include <string>

// This is a spec-reference test for the defaults declared in
// HarnessBridgeConfig.h. We CANNOT include the real header because it
// pulls in Common.h (and transitively the AC type system) — those don't
// exist on the laptop's include path. Instead we re-declare an
// equivalent POD here and assert its defaults match the spec values.
//
// What this test catches: someone changing a spec value (e.g. the
// agreed-on Port number) without updating this test.
// What this test does NOT catch: drift between the real
// HarnessBridgeConfig.h and the re-declaration below. If they diverge,
// the integration test (run on Heimdal during AC build) is what
// surfaces it. Don't trust this test alone to guard the real struct.

namespace HarnessBridge {
    struct Config {
        bool           Enabled            = true;
        std::string    BindAddress        = "127.0.0.1";
        unsigned short Port               = 8091;
        unsigned int   MaxDispatchPerTick = 16;
        unsigned int   QueueCap           = 256;
        unsigned int   RequestTimeoutMs   = 2000;
        std::string    AuditPath          = "/azerothcore/env/dist/logs/harness_bridge.jsonl";
        unsigned int   LoggerLevel        = 3;
    };
}

TEST_CASE("Config defaults match spec §4.2") {
    HarnessBridge::Config c;
    CHECK(c.Enabled == true);
    CHECK(c.BindAddress == "127.0.0.1");
    CHECK(c.Port == 8091);                // spec §3 diagram + §8 port collision fix
    CHECK(c.MaxDispatchPerTick == 16);    // spec §8.4
    CHECK(c.QueueCap == 256);             // spec §8.4
    CHECK(c.RequestTimeoutMs == 2000);    // spec §8.3
    CHECK(c.AuditPath == "/azerothcore/env/dist/logs/harness_bridge.jsonl");
    CHECK(c.LoggerLevel == 3);
}
