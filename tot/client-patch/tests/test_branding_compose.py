# SPDX-License-Identifier: GPL-2.0-or-later
import subprocess
import sys
from pathlib import Path

# parents[0]=tests, parents[1]=client-patch, parents[2]=tot, parents[3]=repo-root
ROOT = Path(__file__).resolve().parents[3]
COMPOSER = ROOT / "tot" / "client-patch" / "compose-tot-addon.py"
ADDON_OUT = ROOT / "tot" / "client-patch" / "build" / "tot-addon" / "ThreadsOfTime"


def test_composer_includes_branding():
    subprocess.run([sys.executable, str(COMPOSER)], check=True, cwd=ROOT)
    composed = list(ADDON_OUT.glob("*ToTBranding*"))
    assert composed, f"branding not composed into {ADDON_OUT}"
    # Composer prefix format: {priority:02d}-{mod_short}-{idx:02d}-{src.name}
    # For mod="tot-branding", priority=5, idx=1, src="ToTBranding.lua":
    #   mod_short = "TotBranding" (no "mod-" prefix stripped; dashes -> PascalCase)
    #   dst = "05-TotBranding-01-ToTBranding.lua"
    assert any(p.name == "05-TotBranding-01-ToTBranding.lua" for p in composed), (
        f"expected '05-TotBranding-01-ToTBranding.lua', got: {[p.name for p in composed]}"
    )


def test_branding_disclaimer_present():
    files = list(ADDON_OUT.glob("*ToTBranding*"))
    assert files, (
        f"branding file missing from {ADDON_OUT} — run test_composer_includes_branding first"
    )
    text = files[0].read_text()
    assert "fan project" in text.lower(), "§10.4 fan-project disclaimer not found in branding Lua"
    assert "@TOT_VERSION@" in text, "@TOT_VERSION@ token must be present at Stage A (substituted at Stage B)"
