#ifndef MOD_HARNESS_BRIDGE_HEALTH_HANDLER_H
#define MOD_HARNESS_BRIDGE_HEALTH_HANDLER_H

namespace httplib { class Server; }

namespace HarnessBridge
{
    // Registers GET /health. Always returns 200 with a tiny JSON body.
    // Used by the daemon to probe AC bridge liveness without going through
    // the tick boundary.
    void RegisterHealthHandler(httplib::Server& server);
}

#endif
