# tot/memory

V3 memory subsystem for Threads of Time bots. Per-bot persistent memory using
SQLite + sqlite-vec, populated via the harness `memory.*` tool surface.

See `docs/superpowers/specs/2026-05-27-tot-1.0.0-v3-memory-subsystem-design.md`
for the schema + retrieval design.

## Run

```
MEMORY_DATA_DIR=./data/memory \
BRAIN_EMBEDDINGS_URL=http://localhost:11434/v1 \
BRAIN_EMBEDDINGS_MODEL=nomic-embed-text \
python -m tot_memory
```

Optional: `BRAIN_EMBEDDINGS_API_KEY=...` if the BYOLLM endpoint requires
bearer-token auth.

The sidecar listens on `0.0.0.0:8090`. A `GET /health` returns
`{"status": "ok"}` once it's up.

## Development

```
cd tot/memory
python -m venv .venv
source .venv/bin/activate
pip install -e '.[test]'
pytest -v
```

## License

GPL-2.0-or-later (matches AzerothCore).
