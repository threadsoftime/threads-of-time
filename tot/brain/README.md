# brain-sidecar

V3 autonomous-decision brain for mod-playerbots living-character bots.

See `docs/superpowers/specs/2026-05-20-thread-f-v3-mvp-brain-sidecar-design.md`.

## Run locally

    pip install -e ".[dev]"
    pytest
    uvicorn brain_sidecar.app:app --host 0.0.0.0 --port 8091

## Deploy to Heimdal

See `deploy/brain-sidecar.container`.
