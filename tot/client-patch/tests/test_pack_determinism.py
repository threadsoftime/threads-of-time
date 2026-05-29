# SPDX-License-Identifier: GPL-2.0-or-later
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
CP = REPO / "tot" / "client-patch"

stormlib = shutil.which("storm") or Path("/opt/homebrew/lib/libstorm.dylib").exists()


@pytest.mark.skipif(not stormlib, reason="StormLib not installed")
def test_pack_is_deterministic():
    subprocess.run([sys.executable, str(CP / "compose-tot-addon.py")], check=True, cwd=REPO)
    r = subprocess.run(
        [sys.executable, str(CP / "pack-mpq.py"), "--version", "1.0.0", "--check"],
        cwd=REPO, capture_output=True, text=True,
    )
    assert r.returncode == 0, r.stderr
    assert "DETERMINISTIC OK" in r.stdout
