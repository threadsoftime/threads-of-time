"""Unit tests for llm_client json_schema response_format support."""
from __future__ import annotations

import pytest

from brain_sidecar.llm_client import LlmClient


@pytest.mark.asyncio
async def test_chat_completion_with_json_schema_sets_response_format(httpx_mock):
    schema = {"oneOf": [{"title": "no_op", "type": "object",
                         "properties": {"kind": {"const": "no_op"}},
                         "required": ["kind"]}]}
    httpx_mock.add_response(
        url="http://llm/v1/chat/completions",
        json={"choices": [{"message": {"content": '{"kind": "no_op"}'}}]},
    )
    client = LlmClient(base_url="http://llm", model="qwen-test", timeout_s=10)
    parsed, raw, latency_ms = await client.chat_completion_json(
        system="s", user="u", json_schema=schema,
    )
    assert parsed == {"kind": "no_op"}
    request = httpx_mock.get_request()
    import json as _json
    body = _json.loads(request.content)
    assert body["response_format"] == {
        "type": "json_schema",
        "json_schema": {"name": "decision", "schema": schema, "strict": True},
    }


@pytest.mark.asyncio
async def test_chat_completion_without_json_schema_still_uses_json_object(httpx_mock):
    httpx_mock.add_response(
        url="http://llm/v1/chat/completions",
        json={"choices": [{"message": {"content": '{"x": 1}'}}]},
    )
    client = LlmClient(base_url="http://llm", model="qwen-test", timeout_s=10)
    parsed, raw, latency_ms = await client.chat_completion_json(system="s", user="u")
    assert parsed == {"x": 1}
    request = httpx_mock.get_request()
    import json as _json
    body = _json.loads(request.content)
    assert body["response_format"] == {"type": "json_object"}


@pytest.mark.asyncio
async def test_chat_completion_falls_back_to_json_object_on_transient_400(httpx_mock):
    schema = {"oneOf": [{"title": "no_op", "type": "object",
                         "properties": {"kind": {"const": "no_op"}},
                         "required": ["kind"]}]}
    # First call: 400 (llama-server rejects json_schema).
    httpx_mock.add_response(
        url="http://llm/v1/chat/completions",
        status_code=400, json={"error": "json_schema unsupported"},
    )
    # Second call (fallback): 200 with json_object.
    httpx_mock.add_response(
        url="http://llm/v1/chat/completions",
        json={"choices": [{"message": {"content": '{"kind": "no_op"}'}}]},
    )
    client = LlmClient(base_url="http://llm", model="qwen-test", timeout_s=10)
    parsed, raw, latency_ms = await client.chat_completion_json(
        system="s", user="u", json_schema=schema,
    )
    assert parsed == {"kind": "no_op"}
    requests = httpx_mock.get_requests()
    assert len(requests) == 2
    import json as _json
    # First request used json_schema; second fell back to json_object.
    assert _json.loads(requests[0].content)["response_format"]["type"] == "json_schema"
    assert _json.loads(requests[1].content)["response_format"]["type"] == "json_object"


@pytest.mark.asyncio
async def test_chat_completion_does_not_fall_back_on_non_400_errors(httpx_mock):
    import httpx
    schema = {"oneOf": [{"title": "no_op", "type": "object", "properties": {"kind": {"const": "no_op"}}, "required": ["kind"]}]}
    httpx_mock.add_response(
        url="http://llm/v1/chat/completions",
        status_code=500, json={"error": "internal"},
    )
    client = LlmClient(base_url="http://llm", model="qwen-test", timeout_s=10)
    with pytest.raises(httpx.HTTPStatusError):
        await client.chat_completion_json(system="s", user="u", json_schema=schema)
