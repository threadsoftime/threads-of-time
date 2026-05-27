#ifndef MOD_HARNESS_BRIDGE_DISPATCH_HANDLER_H
#define MOD_HARNESS_BRIDGE_DISPATCH_HANDLER_H

namespace httplib { class Server; }

namespace HarnessBridge
{
    // Registers POST /dispatch. Parses {tool, args}, enqueues a
    // PendingWork, blocks on future, serializes result.
    void RegisterDispatchHandler(httplib::Server& server);
}

#endif
