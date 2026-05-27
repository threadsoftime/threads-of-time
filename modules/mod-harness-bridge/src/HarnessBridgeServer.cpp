#include "HarnessBridgeServer.h"

#include "Log.h"

// cpp-httplib is vendored under mod-playerbots. We rely on AC's CMake
// adding every modules/*/src include path during MODULES=static build —
// it does, because that's how mod-playerbots itself includes
// Vendor/httplib.h. If that assumption breaks, the include below will
// fail at build time and we add a explicit -I in modules/CMakeLists.txt.
#include "Bot/LlmAgent/Vendor/httplib.h"

namespace HarnessBridge
{
    Server::Server()
        : _server(std::make_unique<httplib::Server>())
    {
    }

    Server::~Server()
    {
        Stop();
    }

    httplib::Server& Server::Raw()
    {
        return *_server;
    }

    bool Server::Start(std::string const& bind_addr, uint16_t port)
    {
        if (_running.load())
        {
            LOG_WARN("server.loading", "[mod-harness-bridge] Start() called while already running");
            return true;
        }

        if (!_server->bind_to_port(bind_addr.c_str(), port))
        {
            LOG_ERROR("server.loading",
                      "[mod-harness-bridge] bind_to_port({}:{}) failed", bind_addr, port);
            return false;
        }

        _running.store(true);
        _thread = std::thread([this, bind_addr, port]()
        {
            LOG_INFO("server.loading",
                     "[mod-harness-bridge] listen loop started on {}:{}", bind_addr, port);
            _server->listen_after_bind();
            _running.store(false);
            LOG_INFO("server.loading", "[mod-harness-bridge] listen loop exited");
        });

        return true;
    }

    void Server::Stop()
    {
        if (!_running.load() && !_thread.joinable())
            return;

        if (_server)
            _server->stop();

        if (_thread.joinable())
            _thread.join();

        _running.store(false);
    }
}
