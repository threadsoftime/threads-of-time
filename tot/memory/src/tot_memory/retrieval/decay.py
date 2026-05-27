# SPDX-License-Identifier: GPL-2.0-or-later
"""Exponential time-decay for episodes, per design subspec §7.

Weight at age ``Δt`` (hours) with half-life ``h`` (hours):

    w(Δt) = 2 ** (-Δt / h)

→ ``w(0) = 1.0`` (fresh), ``w(h) = 0.5``, ``w(∞) → 0``.

Future timestamps (e.g. clock skew between the game server and the memory
sidecar) are clamped so they cannot produce ``w > 1``.

§7.2 specifies per-episode-type half-lives in seconds; this module exposes
them in hours because the public ``decay_weight`` signature takes
``half_life_hours`` (matches the plan-template's Task 15 contract). All
values are configurable via env vars at run time — see ``config.py``.
"""

import math
from datetime import datetime

# Per design subspec §7.2 — default half-lives in HOURS.
# Values converted from the §7.2 second-form for readability:
#   chat        48h    (172800s)
#   combat      72h    (259200s)
#   social      96h    (345600s)
#   quest      168h    (604800s — 7 days)
#   discovery  720h    (2592000s — 30 days)
#   goal       168h    (604800s — 7 days)
#   reflection 336h    (1209600s — 14 days)
#   observation 24h    (86400s)
HALF_LIVES_HOURS: dict[str, float] = {
    "chat":        48.0,
    "combat":      72.0,
    "social":      96.0,
    "quest":      168.0,
    "discovery":  720.0,
    "goal":       168.0,
    "reflection": 336.0,
    "observation": 24.0,
}

# Fallback when an unknown episode_type is supplied. The conservative choice
# is the shortest sensible default (chat — 48h); aging fast is safer than
# pinning irrelevant memories at the top of recall.
_FALLBACK_TYPE = "chat"


def half_life_for(episode_type: str) -> float:
    """Return the configured half-life in hours for ``episode_type``.

    Unknown types fall back to the ``chat`` half-life (48 h) so the
    caller always gets a usable value without raising.
    """
    return HALF_LIVES_HOURS.get(episode_type.lower(), HALF_LIVES_HOURS[_FALLBACK_TYPE])


def decay_weight(
    episode_time: datetime,
    now: datetime,
    half_life_hours: float,
) -> float:
    """Exponential decay multiplier in ``(0, 1]``.

    Args:
        episode_time: Event time of the episode (timezone-aware datetime).
        now:          Current wall-clock time.
        half_life_hours: Half-life in hours — the age at which decay = 0.5.
            Must be strictly positive.

    Returns:
        ``2 ** (-age_hours / half_life_hours)`` — clamped so that future
        timestamps (negative age) cannot exceed 1.0.

    Raises:
        ValueError: when ``half_life_hours <= 0``.
    """
    if half_life_hours <= 0:
        raise ValueError(f"half_life_hours must be positive, got {half_life_hours}")
    age_seconds = (now - episode_time).total_seconds()
    if age_seconds <= 0:
        return 1.0
    age_hours = age_seconds / 3600.0
    return math.pow(0.5, age_hours / half_life_hours)
