# Threads of Time — BYOLLM Setup

Threads of Time's decision brain calls two LLM endpoints: one for chat completions
(bot reasoning/dialogue) and one for text embeddings (memory retrieval). Both must
be **OpenAI-compatible** (i.e., expose `/v1/chat/completions` and `/v1/embeddings`).
They may be the same server if it supports both APIs.

The relevant `.env` variables:

| Variable | Role |
|---|---|
| `BRAIN_LLM_URL` | Base URL for the chat endpoint (e.g. `http://192.168.1.50:11434/v1`) |
| `BRAIN_LLM_MODEL` | Chat model name passed in `"model"` field |
| `BRAIN_LLM_API_KEY` | API key if required (leave blank for local servers) |
| `BRAIN_EMBEDDINGS_URL` | Base URL for the embeddings endpoint |
| `BRAIN_EMBEDDINGS_MODEL` | Embeddings model name (e.g. `nomic-embed-text`) |
| `BRAIN_EMBEDDINGS_API_KEY` | API key if required (leave blank for local servers) |

---

## Option A: Ollama (recommended for local operators)

Ollama exposes an OpenAI-compatible API on port `11434`. Both chat and embeddings
can run on the same Ollama server.

```sh
# Pull the models:
ollama pull qwen2.5:14b-instruct
ollama pull nomic-embed-text

# Confirm they're loaded:
ollama list
```

Then in `$TOT_HOME/.env` (replace `192.168.1.50` with your Ollama host IP or
`127.0.0.1` if Ollama runs on the same machine as the ToT stack):

```bash
BRAIN_LLM_URL=http://192.168.1.50:11434/v1
BRAIN_LLM_MODEL=qwen2.5:14b-instruct
BRAIN_LLM_API_KEY=

BRAIN_EMBEDDINGS_URL=http://192.168.1.50:11434/v1
BRAIN_EMBEDDINGS_MODEL=nomic-embed-text
BRAIN_EMBEDDINGS_API_KEY=
```

Restart the brain after updating: `tot restart brain`.

**Resource guidance:** `qwen2.5:14b-instruct` (Q4_K_M, ~9 GB) fits in 12 GB VRAM.
For systems with less VRAM, try `qwen2.5:7b-instruct` (~5 GB). `nomic-embed-text`
is small (~270 MB) and can run CPU-only without noticeable latency.

---

## Option B: llama.cpp / llama-server

```sh
# Chat endpoint (adjust model path and port):
llama-server --model /models/qwen2.5-14b-instruct-q4_k_m.gguf --port 8080 --host 0.0.0.0

# Embeddings endpoint (separate process, separate port):
llama-server --model /models/nomic-embed-text-v1.5.f32.gguf --port 8081 --host 0.0.0.0 --embedding
```

Then in `.env`:

```bash
BRAIN_LLM_URL=http://<host>:8080/v1
BRAIN_LLM_MODEL=qwen2.5-14b-instruct-q4_k_m

BRAIN_EMBEDDINGS_URL=http://<host>:8081/v1
BRAIN_EMBEDDINGS_MODEL=nomic-embed-text-v1.5.f32
```

---

## Option C: Hosted API (OpenRouter / OpenAI)

```bash
BRAIN_LLM_URL=https://openrouter.ai/api/v1
BRAIN_LLM_MODEL=qwen/qwen-2.5-14b-instruct
BRAIN_LLM_API_KEY=sk-or-<your-key>

BRAIN_EMBEDDINGS_URL=https://api.openai.com/v1
BRAIN_EMBEDDINGS_MODEL=text-embedding-3-small
BRAIN_EMBEDDINGS_API_KEY=sk-<your-key>
```

Note: hosted APIs introduce latency (~200–500 ms per decision vs. ~50–100 ms local).
Bot response frequency is tuned for local inference; hosted APIs still work but bots
will react more slowly.

---

## Advanced: multi-endpoint routing (LiteLLM proxy)

ToT ships a **single-endpoint** design for simplicity: one `BRAIN_LLM_URL` and one
`BRAIN_EMBEDDINGS_URL`. Multi-endpoint fallback routing (primary/secondary LLM with
automatic failover) is intentionally not a built-in ToT feature — keeping the brain
config surface small makes the system easier to operate and debug.

If you need fallback routing (e.g. primary GPU server + cloud fallback), put a proxy
in front and point `BRAIN_LLM_URL` at it. [LiteLLM](https://github.com/BerriAI/litellm)
works well:

```sh
pip install litellm[proxy]
litellm --model qwen2.5:14b-instruct --fallbacks '[{"model": "openai/gpt-4o-mini"}]' --port 4000
```

Then: `BRAIN_LLM_URL=http://127.0.0.1:4000/v1`.

---

## Verifying your LLM endpoint

Before starting the stack, confirm the endpoints respond:

```sh
# Chat models list:
curl $BRAIN_LLM_URL/models

# Test chat completion:
curl -s -X POST $BRAIN_LLM_URL/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "'"$BRAIN_LLM_MODEL"'", "messages": [{"role": "user", "content": "ping"}]}' \
  | python3 -m json.tool | grep '"content"'

# Test embeddings:
curl -s -X POST $BRAIN_EMBEDDINGS_URL/embeddings \
  -H "Content-Type: application/json" \
  -d '{"model": "'"$BRAIN_EMBEDDINGS_MODEL"'", "input": "hello"}' \
  | python3 -m json.tool | grep '"object"'
```

Once the stack is running, a working LLM connection is confirmed by bots greeting
players in-game within one tick interval (~60 seconds after first player login).
