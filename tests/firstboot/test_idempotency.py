# tests/firstboot/test_idempotency.py
import subprocess, os, stat
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[2] / "tot/deploy/firstboot/firstboot.sh"

def test_flag_present_short_circuits(tmp_path):
    data = tmp_path / "data"; data.mkdir()
    (data / "bootstrap.flag").write_text("2026-01-01T00:00:00Z\n")
    # Run with a doctored env where the flag dir is tmp; the script must exit 0
    # at the flag check BEFORE touching MySQL (no mysqladmin needed).
    env = {**os.environ, "PATH": os.environ["PATH"]}
    # Patch FLAG path by running a wrapper that overrides /tot/data via sed-free bind:
    wrapper = tmp_path / "run.sh"
    wrapper.write_text(
        f'#!/usr/bin/env bash\nset -e\nsed "s#/tot/data#{data}#g" "{SCRIPT}" > "{tmp_path}/fb.sh"\n'
        f'bash "{tmp_path}/fb.sh"\n'
    )
    wrapper.chmod(wrapper.stat().st_mode | stat.S_IEXEC)
    r = subprocess.run(["bash", str(wrapper)], capture_output=True, text=True)
    assert r.returncode == 0
    assert "skipping" in r.stdout
