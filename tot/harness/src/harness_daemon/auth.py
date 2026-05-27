"""Bearer auth + scope-glob + self-binding checks.

Spec §7.3 + §7.4. Subject-GUID enforcement (step 8 of the auth flow)
lives in the registry path — auth.py only exposes the primitives.
"""

from __future__ import annotations

import fnmatch
from dataclasses import dataclass
from typing import Iterable, Optional

from .config import TokenRecord


class AuthError(Exception):
    """Auth failure — maps to HTTP 401 or 403 at the FastAPI layer."""

    def __init__(self, code: str, detail: str = "") -> None:
        super().__init__(code)
        self.code = code
        self.detail = detail


@dataclass(frozen=True)
class AuthResult:
    """The token's identity + scope + binding, returned to handlers."""
    identity:       str
    scope:          list[str]
    bound_to_guid:  Optional[int]
    augmented:      bool


class TokenStore:
    """O(1) lookup over a list of TokenRecord."""

    def __init__(self, tokens: Iterable[TokenRecord]) -> None:
        self._by_token = {t.token: t for t in tokens}

    def find(self, token: str) -> Optional[TokenRecord]:
        return self._by_token.get(token)


def authenticate_bearer(store: TokenStore, header_value: Optional[str]) -> AuthResult:
    """Parse `Authorization: Bearer <token>` and resolve the TokenRecord.

    Raises AuthError('unauthorized') for any failure — never leaks
    whether the token was unknown vs. malformed vs. missing.
    """
    if not header_value:
        raise AuthError("unauthorized", "missing bearer")
    prefix = "Bearer "
    if not header_value.startswith(prefix):
        raise AuthError("unauthorized", "malformed authorization header")
    token = header_value[len(prefix):].strip()
    if not token:
        raise AuthError("unauthorized", "empty token")

    record = store.find(token)
    if record is None:
        raise AuthError("unauthorized", "unknown token")

    return AuthResult(
        identity=record.identity,
        scope=list(record.scope),
        bound_to_guid=record.bound_to_guid,
        augmented=record.augmented,
    )


def is_self_scope(pattern: str) -> bool:
    """True iff this scope pattern requires subject-GUID self-binding.

    Patterns shaped like `<ns>.self.*` are structural — they also enforce
    `subject_guid == token.bound_to_guid` (callers handle that check).
    """
    parts = pattern.split(".")
    return len(parts) >= 2 and parts[1] == "self"


def _pattern_matches(pattern: str, tool: str) -> bool:
    """fnmatch-style glob constrained to single-segment '*' semantics.

    Convert `gm.*` → match exactly `gm.X` (one segment). Convert
    `bot.self.*` → match `bot.X` (the `.self.` infix is structural; for
    glob-match purposes we collapse to `bot.*`).
    """
    if "*" not in pattern:
        return pattern == tool

    # Collapse `<ns>.self.*` to `<ns>.*` for matching.
    if is_self_scope(pattern):
        parts = pattern.split(".", 2)
        # parts = ['bot', 'self', '*']
        normalized = parts[0] + "." + parts[2]
    else:
        normalized = pattern

    # Enforce single-segment '*' (fnmatch's '*' would cross dots).
    # `gm.*` → split into prefix `gm.` + suffix glob `*` over one segment.
    if normalized.endswith(".*"):
        prefix = normalized[:-2] + "."
        if not tool.startswith(prefix):
            return False
        suffix = tool[len(prefix):]
        return suffix != "" and "." not in suffix

    # Other glob forms not used in V1.
    return fnmatch.fnmatchcase(tool, normalized)


def scope_allows(scope: Iterable[str], tool: str) -> bool:
    """True iff at least one pattern in `scope` matches `tool`."""
    for pattern in scope:
        if _pattern_matches(pattern, tool):
            return True
    return False
