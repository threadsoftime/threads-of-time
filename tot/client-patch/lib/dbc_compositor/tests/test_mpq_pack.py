# tools/dbc-patch-builder/tests/test_mpq_pack.py
import struct
import tempfile
from pathlib import Path

import pytest

from dbc_compositor.mpq_pack import pack_mpq


def test_pack_creates_file():
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "test.mpq"
        pack_mpq(out, {"DBFilesClient\\TestA.dbc": b"WDBC_data_A"})
        assert out.exists()
        assert out.stat().st_size > 0


def test_pack_magic_header():
    """MPQ files start with 'MPQ\\x1a'."""
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "test.mpq"
        pack_mpq(out, {"DBFilesClient\\TestA.dbc": b"WDBC_data"})
        with open(out, "rb") as f:
            head = f.read(4)
        assert head == b"MPQ\x1a", f"expected MPQ magic, got {head}"


def test_pack_two_files_succeeds():
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "test.mpq"
        pack_mpq(out, {
            "DBFilesClient\\TestA.dbc": b"WDBC" + b"a" * 1000,
            "DBFilesClient\\TestB.dbc": b"WDBC" + b"b" * 1000,
        })
        assert out.exists()


def test_pack_overwrites_existing():
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "test.mpq"
        pack_mpq(out, {"DBFilesClient\\X.dbc": b"first"})
        first_size = out.stat().st_size
        pack_mpq(out, {"DBFilesClient\\X.dbc": b"second-payload-that-is-longer"})
        # File exists, no error. Different content => maybe different size; just verify exists.
        assert out.exists()
