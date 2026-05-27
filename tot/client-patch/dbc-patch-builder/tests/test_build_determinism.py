# tools/dbc-patch-builder/tests/test_build_determinism.py
import hashlib
import tempfile
from pathlib import Path
from dbc_patch_builder.build import main as build_main


def test_two_runs_produce_identical_output():
    with tempfile.TemporaryDirectory() as td:
        a = Path(td) / "a.mpq"
        b = Path(td) / "b.mpq"
        build_main(["--out", str(a)])
        build_main(["--out", str(b)])
        sha_a = hashlib.sha256(a.read_bytes()).hexdigest()
        sha_b = hashlib.sha256(b.read_bytes()).hexdigest()
        assert sha_a == sha_b, f"non-deterministic: {sha_a} vs {sha_b}"
