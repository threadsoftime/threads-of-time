"""Minimal mock AC bridge for parity testing.

Serves two routes:
  POST /dispatch  → {"ok":true,"result":{"tool":<tool>,"args":<args>}} (200)
  GET  /health    → 200

Supports:
  - set_fault(tool_name, status_code): next call to that tool returns that status
  - set_unreachable(True): stop accepting connections (simulate AC down)
  - stop(): clean shutdown
"""
from __future__ import annotations

import json
import socket
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Optional


class _Handler(BaseHTTPRequestHandler):
    """Single-threaded request handler wired to the MockAC singleton."""

    # Silence the default "GET /foo HTTP/1.1 200 -" log lines in test output.
    def log_message(self, fmt, *args):  # type: ignore[override]
        pass

    def _send_json(self, status: int, body: dict) -> None:
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/health":
            self._send_json(200, {"ok": True})
        else:
            self._send_json(404, {"ok": False, "error": "not_found"})

    def do_POST(self) -> None:  # noqa: N802
        server: MockAC = self.server  # type: ignore[assignment]

        if self.path != "/dispatch":
            self._send_json(404, {"ok": False, "error": "not_found"})
            return

        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b"{}"
        try:
            req = json.loads(raw)
        except json.JSONDecodeError:
            self._send_json(400, {"ok": False, "error": "bad_json"})
            return

        tool: str = req.get("tool", "unknown")

        with server.lock:
            fault = server.faults.get(tool)

        if fault is not None:
            self._send_json(fault, {"ok": False, "error": "fault_injected"})
            return

        self._send_json(200, {"ok": True, "result": {"tool": tool, "args": req.get("args", {})}})


class MockAC(HTTPServer):
    """Threaded mock AC bridge bound to an ephemeral port."""

    def __init__(self) -> None:
        # Bind to port 0 → OS assigns ephemeral port.
        super().__init__(("127.0.0.1", 0), _Handler)
        self.lock: threading.Lock = threading.Lock()
        self.faults: dict[str, int] = {}
        self._thread: Optional[threading.Thread] = None

    @property
    def base_url(self) -> str:
        host, port = self.server_address
        return f"http://{host}:{port}"

    def set_fault(self, tool: str, status: int) -> None:
        """Next call to `tool` returns `status` instead of 200."""
        with self.lock:
            self.faults[tool] = status

    def clear_fault(self, tool: str) -> None:
        with self.lock:
            self.faults.pop(tool, None)

    def start(self) -> None:
        """Start serving in a background daemon thread."""
        self._thread = threading.Thread(target=self.serve_forever, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        """Shut down the server; join the thread."""
        self.shutdown()
        if self._thread is not None:
            self._thread.join(timeout=5)


def start_mock_ac() -> MockAC:
    """Create, start, and return a running MockAC on an ephemeral port."""
    mock = MockAC()
    mock.start()
    return mock
