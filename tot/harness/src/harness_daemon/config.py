"""YAML config loader and validation for harness-daemon.

See spec §7.1 for the full schema. Key invariants:
- `augmented` tokens are capped at `max_augmented_bots` (default 1).
- An `augmented` token MUST have `bound_to_guid` set.
- Token strings must be unique.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import yaml


class ConfigError(ValueError):
    """Raised when the config is malformed or violates an invariant."""


@dataclass(frozen=True)
class TokenRecord:
    token:          str
    identity:       str
    scope:          list[str]
    augmented:      bool             = False
    bound_to_guid:  Optional[int]    = None
    note:           Optional[str]    = None


@dataclass(frozen=True)
class DaemonConfig:
    max_augmented_bots: int
    ac_bridge_url:      str
    audit_path:         str
    listen_address:     str
    tokens:             list[TokenRecord] = field(default_factory=list)


def _require(d: dict, key: str, ctx: str) -> object:
    if key not in d:
        raise ConfigError(f"missing required key '{key}' in {ctx}")
    return d[key]


def _parse_token(raw: dict, idx: int) -> TokenRecord:
    ctx = f"tokens[{idx}]"
    if not isinstance(raw, dict):
        raise ConfigError(f"{ctx} must be a mapping, got {type(raw).__name__}")

    token = _require(raw, "token", ctx)
    identity = _require(raw, "identity", ctx)
    scope = _require(raw, "scope", ctx)

    if not isinstance(token, str) or not token:
        raise ConfigError(f"{ctx}.token must be a non-empty string")
    if not isinstance(identity, str) or not identity:
        raise ConfigError(f"{ctx}.identity must be a non-empty string")
    if not isinstance(scope, list) or not all(isinstance(s, str) for s in scope):
        raise ConfigError(f"{ctx}.scope must be a list of strings")

    augmented = bool(raw.get("augmented", False))
    bound_to_guid = raw.get("bound_to_guid")
    if bound_to_guid is not None and not isinstance(bound_to_guid, int):
        raise ConfigError(f"{ctx}.bound_to_guid must be an integer (or omitted)")
    if augmented and bound_to_guid is None:
        raise ConfigError(f"{ctx}: augmented=true requires bound_to_guid")

    note = raw.get("note")
    if note is not None and not isinstance(note, str):
        raise ConfigError(f"{ctx}.note must be a string (or omitted)")

    return TokenRecord(
        token=token,
        identity=identity,
        scope=list(scope),
        augmented=augmented,
        bound_to_guid=bound_to_guid,
        note=note,
    )


def load_config_from_dict(raw: dict) -> DaemonConfig:
    """Parse a config dict (already-loaded YAML) into DaemonConfig."""
    if not isinstance(raw, dict):
        raise ConfigError(f"top-level config must be a mapping, got {type(raw).__name__}")

    max_aug = raw.get("max_augmented_bots", 1)
    if not isinstance(max_aug, int) or max_aug < 0:
        raise ConfigError("max_augmented_bots must be a non-negative integer")

    ac_url = _require(raw, "ac_bridge_url", "top-level")
    audit = _require(raw, "audit_path", "top-level")
    listen = _require(raw, "listen_address", "top-level")
    if not all(isinstance(v, str) for v in (ac_url, audit, listen)):
        raise ConfigError("ac_bridge_url / audit_path / listen_address must be strings")

    raw_tokens = raw.get("tokens", [])
    if not isinstance(raw_tokens, list):
        raise ConfigError("tokens must be a list")

    parsed = [_parse_token(t, i) for i, t in enumerate(raw_tokens)]

    # Uniqueness
    seen: set[str] = set()
    for t in parsed:
        if t.token in seen:
            raise ConfigError(f"duplicate token: '{t.token}'")
        seen.add(t.token)

    # Augmented-bot cap
    aug_count = sum(1 for t in parsed if t.augmented)
    if aug_count > max_aug:
        offenders = [t.identity for t in parsed if t.augmented]
        raise ConfigError(
            f"augmented-bot cap exceeded: {aug_count} > max_augmented_bots={max_aug}; "
            f"offending identities: {offenders}"
        )

    return DaemonConfig(
        max_augmented_bots=max_aug,
        ac_bridge_url=ac_url,
        audit_path=audit,
        listen_address=listen,
        tokens=parsed,
    )


def load_config_from_path(path: Path | str) -> DaemonConfig:
    """Read YAML from disk and parse."""
    p = Path(path)
    with p.open("r") as fh:
        raw = yaml.safe_load(fh) or {}
    return load_config_from_dict(raw)
