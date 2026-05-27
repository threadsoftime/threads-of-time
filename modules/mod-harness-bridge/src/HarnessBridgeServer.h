#ifndef MOD_HARNESS_BRIDGE_SERVER_H
#define MOD_HARNESS_BRIDGE_SERVER_H

#include <atomic>
#include <memory>
#include <thread>

namespace httplib { class Server; }

namespace HarnessBridge
{
    // Owns the cpp-httplib::Server and the thread that runs its accept
    // loop. Start() spawns the thread and binds; Stop() shuts the server
    // down and joins the thread.
    //
    // Handler registration happens before Start() in HarnessBridge.cpp's
    // OnStartup(). After Start() the server is read-only.

    class Server
    {
    public:
        Server();
        ~Server();

        Server(Server const&) = delete;
        Server& operator=(Server const&) = delete;

        // Returns the underlying httplib::Server for handler registration.
        // Only safe to call before Start().
        httplib::Server& Raw();

        // Binds + spawns the accept loop. Returns false on bind failure.
        bool Start(std::string const& bind_addr, uint16_t port);

        // Idempotent. Safe to call from WorldScript::OnShutdown.
        void Stop();

        bool IsRunning() const { return _running.load(); }

    private:
        std::unique_ptr<httplib::Server> _server;
        std::thread                      _thread;
        std::atomic<bool>                _running { false };
    };
}

#endif // MOD_HARNESS_BRIDGE_SERVER_H
