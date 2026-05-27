"""V3.6: BRAIN_MAX_PLAYER_LEVEL env var support."""
from __future__ import annotations

from brain_sidecar.settings import get_settings


def test_max_player_level_default_is_25(monkeypatch):
    """Default cap matches the Bracket 1 MaxPlayerLevel in worldserver.conf (L25)."""
    monkeypatch.delenv("BRAIN_MAX_PLAYER_LEVEL", raising=False)
    settings = get_settings()
    assert settings.max_player_level == 25


def test_max_player_level_env_override(monkeypatch):
    """Operator can bump the cap (e.g., for Bracket 2 testing at L35)."""
    monkeypatch.setenv("BRAIN_MAX_PLAYER_LEVEL", "35")
    settings = get_settings()
    assert settings.max_player_level == 35
