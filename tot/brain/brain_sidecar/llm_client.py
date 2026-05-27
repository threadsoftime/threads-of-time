"""Thin async HTTP client for the Qwen llama-server OpenAI-compatible API."""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

import httpx


@dataclass
class LlmClient:
    base_url: str
    model: str
    timeout_s: float

    async def chat_completion_json(
        self,
        *,
        system: str,
        user: str,
        max_tokens: int = 400,
        temperature: float = 0.5,
        json_schema: dict[str, Any] | None = None,
    ) -> tuple[dict[str, Any] | None, str, float]:
        """Call /v1/chat/completions with a structured response_format.

        When `json_schema` is provided, llama-server enforces the response with
        grammar derived from the schema (V3.1 path). When omitted, falls back
        to free-form json_object mode (V3-MVP back-compat for tests).

        Returns (parsed_json | None, raw_text, latency_ms).
        """
        import time

        if json_schema is not None:
            response_format: dict[str, Any] = {
                "type": "json_schema",
                "json_schema": {"name": "decision", "schema": json_schema, "strict": True},
            }
        else:
            response_format = {"type": "json_object"}

        payload = {
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "max_tokens": max_tokens,
            "temperature": temperature,
            "response_format": response_format,
        }
        t0 = time.monotonic()
        async with httpx.AsyncClient(timeout=self.timeout_s) as client:
            r = await client.post(f"{self.base_url}/v1/chat/completions", json=payload)
            # Transient json_schema rejection: fall back ONCE to json_object.
            if r.status_code == 400 and json_schema is not None:
                import logging
                logging.getLogger(__name__).warning(
                    "llm_client_json_schema_rejected_falling_back status=%s body=%s",
                    r.status_code, r.text[:200],
                )
                fallback_payload = dict(payload)
                fallback_payload["response_format"] = {"type": "json_object"}
                r = await client.post(f"{self.base_url}/v1/chat/completions", json=fallback_payload)
            r.raise_for_status()
            data = r.json()
        latency_ms = (time.monotonic() - t0) * 1000.0
        raw = data["choices"][0]["message"]["content"]
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError:
            parsed = None
        return parsed, raw, latency_ms
