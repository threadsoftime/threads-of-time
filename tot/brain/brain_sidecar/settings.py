"""Env-based configuration for brain-sidecar."""
from __future__ import annotations

import os
from dataclasses import dataclass


@dataclass(frozen=True)
class Settings:
    # Service
    bind_host: str
    bind_port: int
    # State storage
    state_db_path: str
    # Logging
    decisions_log_path: str
    # Upstream MCPs (prefixed with the upstream service's name, not ours)
    harness_mcp_url: str
    memory_mcp_url: str
    harness_bearer: str
    memory_bearer: str
    # LLM
    llm_base_url: str
    llm_model: str
    llm_timeout_s: float
    # Brain auth (incoming). Empty string means: auth disabled (dev mode).
    brain_bearer: str
    # Decision loop
    tick_interval_s: float
    # Personality cache
    personality_ttl_s: float
    # SSE consumer
    brain_sse_enabled: bool
    brain_sse_coalesce_ms: int
    brain_sse_dedup_capacity: int
    # V3.6: at-cap derivation cap
    max_player_level: int


def _parse_bool(val: str) -> bool:
    """Parse a truthy env-var string into bool."""
    return val.strip().lower() in ("1", "true", "yes")


def get_settings() -> Settings:
    """Read live environment on each call. Tests can monkeypatch env vars."""
    return Settings(
        bind_host=os.getenv("BRAIN_BIND_HOST", "0.0.0.0"),
        bind_port=int(os.getenv("BRAIN_BIND_PORT", "8091")),
        state_db_path=os.getenv("BRAIN_STATE_DB", "/opt/containers/brain/state.sqlite"),
        decisions_log_path=os.getenv(
            "BRAIN_DECISIONS_LOG", "/opt/containers/brain/logs/decisions.jsonl"
        ),
        harness_mcp_url=os.getenv("HARNESS_MCP_URL", "http://localhost:8099/mcp/mcp"),
        memory_mcp_url=os.getenv("MEMORY_MCP_URL", "http://localhost:8090/mcp/mcp"),
        harness_bearer=os.getenv("HARNESS_BEARER", ""),
        memory_bearer=os.getenv("MEMORY_BEARER", ""),
        llm_base_url=os.getenv("LLM_BASE_URL", "http://localhost:8080"),
        llm_model=os.getenv("LLM_MODEL", "qwen2.5-7b-instruct"),
        llm_timeout_s=float(os.getenv("LLM_TIMEOUT_S", "60")),
        brain_bearer=os.getenv("BRAIN_BEARER", ""),
        tick_interval_s=float(os.getenv("BRAIN_TICK_INTERVAL_S", "5")),
        personality_ttl_s=float(os.getenv("PERSONALITY_TTL_S", "300")),
        brain_sse_enabled=_parse_bool(os.getenv("BRAIN_SSE_ENABLED", "1")),
        brain_sse_coalesce_ms=int(os.getenv("BRAIN_SSE_COALESCE_MS", "200")),
        brain_sse_dedup_capacity=int(os.getenv("BRAIN_SSE_DEDUP_CAPACITY", "100")),
        max_player_level=int(os.getenv("BRAIN_MAX_PLAYER_LEVEL", "25")),
    )
