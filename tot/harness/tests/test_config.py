"""Tests for the YAML config loader + cap enforcement."""

from __future__ import annotations

import pytest
import yaml

from harness_daemon.config import (
    DaemonConfig,
    TokenRecord,
    ConfigError,
    load_config_from_dict,
    load_config_from_path,
)


def test_load_minimal_config(example_config_dict: dict) -> None:
    cfg = load_config_from_dict(example_config_dict)
    assert cfg.max_augmented_bots == 1
    assert cfg.ac_bridge_url == "http://127.0.0.1:8091"
    assert cfg.listen_address == "0.0.0.0:8090"
    assert len(cfg.tokens) == 1
    assert cfg.tokens[0].identity == "test.runner"
    assert cfg.tokens[0].scope == ["gm.*", "obs.*"]
    assert cfg.tokens[0].augmented is False
    assert cfg.tokens[0].bound_to_guid is None


def test_load_from_yaml_file(tmp_path, example_config_dict: dict) -> None:
    p = tmp_path / "tokens.yaml"
    p.write_text(yaml.safe_dump(example_config_dict))
    cfg = load_config_from_path(p)
    assert cfg.tokens[0].token == "tok-test-123"


def test_augmented_cap_rejects_excess(example_config_dict: dict) -> None:
    example_config_dict["max_augmented_bots"] = 1
    example_config_dict["tokens"] = [
        {"token": "t1", "identity": "bot.A", "scope": ["bot.self.*"],
         "augmented": True, "bound_to_guid": 1},
        {"token": "t2", "identity": "bot.B", "scope": ["bot.self.*"],
         "augmented": True, "bound_to_guid": 2},
    ]
    with pytest.raises(ConfigError, match="augmented"):
        load_config_from_dict(example_config_dict)


def test_augmented_cap_allows_below(example_config_dict: dict) -> None:
    example_config_dict["max_augmented_bots"] = 2
    example_config_dict["tokens"] = [
        {"token": "t1", "identity": "bot.A", "scope": ["bot.self.*"],
         "augmented": True, "bound_to_guid": 1},
    ]
    cfg = load_config_from_dict(example_config_dict)
    assert sum(1 for t in cfg.tokens if t.augmented) == 1


def test_augmented_requires_bound_to_guid(example_config_dict: dict) -> None:
    example_config_dict["tokens"] = [
        {"token": "t1", "identity": "bot.A", "scope": ["bot.self.*"],
         "augmented": True},  # missing bound_to_guid
    ]
    with pytest.raises(ConfigError, match="bound_to_guid"):
        load_config_from_dict(example_config_dict)


def test_duplicate_tokens_rejected(example_config_dict: dict) -> None:
    example_config_dict["tokens"] = [
        {"token": "same", "identity": "a", "scope": ["gm.*"]},
        {"token": "same", "identity": "b", "scope": ["gm.*"]},
    ]
    with pytest.raises(ConfigError, match="duplicate"):
        load_config_from_dict(example_config_dict)


def test_missing_required_fields(example_config_dict: dict) -> None:
    example_config_dict["tokens"] = [{"identity": "x", "scope": ["gm.*"]}]
    with pytest.raises(ConfigError, match="token"):
        load_config_from_dict(example_config_dict)
