# tools/dbc-patch-builder/src/dbc_patch_builder/check.py
"""Drift detector: re-run build, compare sha256 to a recorded value."""
from __future__ import annotations
import argparse
import hashlib
import sys
import tempfile
from pathlib import Path

from .build import main as build_main


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="Verify patch-Z.MPQ matches the expected sha256 (fixture drift detector)")
    parser.add_argument("--expected-sha256", required=True, help="Expected sha256 of patch-Z.MPQ")
    args = parser.parse_args(argv)

    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "patch-Z.MPQ"
        build_main(["--out", str(out)])
        actual = hashlib.sha256(out.read_bytes()).hexdigest()

    if actual != args.expected_sha256:
        print(f"DRIFT DETECTED")
        print(f"  expected: {args.expected_sha256}")
        print(f"  actual:   {actual}")
        return 1
    print(f"OK: sha256 = {actual}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
