"""Smoke test that contract tier skips cleanly without bearers."""
from __future__ import annotations

import pytest


@pytest.mark.contract
async def test_contract_marker_active(harness_bearer):
    # If we reach this line, the fixture provided a bearer.
    # If bearers were absent, the fixture would have called pytest.skip.
    assert harness_bearer
