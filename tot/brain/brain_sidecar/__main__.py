"""Run via `python -m brain_sidecar`."""
from __future__ import annotations

import uvicorn

from brain_sidecar.settings import get_settings


def main() -> None:
    s = get_settings()
    uvicorn.run("brain_sidecar.app:create_app", factory=True, host=s.bind_host, port=s.bind_port, log_level="info")


if __name__ == "__main__":
    main()
