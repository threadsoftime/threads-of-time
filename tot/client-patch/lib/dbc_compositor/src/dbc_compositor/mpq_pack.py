# tools/dbc-patch-builder/src/dbc_patch_builder/mpq_pack.py
"""Pack DBC files into a patch MPQ via StormLib (loaded as a shared library via ctypes).

StormLib is the standard MPQ library by Ladislav Zezula. On macOS install
via `brew install stormlib`; on Linux build from source per
https://github.com/ladislav-zezula/StormLib.

This module is intended for use on a workstation. CI gates that need to
regenerate the patch should ensure StormLib is available in their build
environment.
"""
from __future__ import annotations
import ctypes
import os
import sys
import tempfile
from ctypes import c_char_p, c_int, c_uint, c_void_p, POINTER, byref
from pathlib import Path
from typing import Dict


# MPQ_CREATE_ARCHIVE_V1 = 0x00000000 (default v1 archive — what WoW 3.3.5a uses)
MPQ_CREATE_ARCHIVE_V1 = 0x00000000

# MPQ_FILE_COMPRESS = 0x00000200 (zlib compression)
MPQ_FILE_COMPRESS = 0x00000200

# MPQ_COMPRESSION_ZLIB = 0x02
MPQ_COMPRESSION_ZLIB = 0x02


def _find_stormlib() -> str:
    """Locate libstorm.dylib (macOS) or libstorm.so (Linux)."""
    if sys.platform == "darwin":
        candidates = [
            "/opt/homebrew/lib/libstorm.dylib",
            "/opt/homebrew/Cellar/stormlib/9.31/lib/libstorm.dylib",
            "/usr/local/lib/libstorm.dylib",
        ]
    else:
        candidates = [
            "/usr/lib/libstorm.so",
            "/usr/lib/x86_64-linux-gnu/libstorm.so",
            "/usr/local/lib/libstorm.so",
        ]
    for c in candidates:
        if Path(c).exists():
            return c
        # Symlink resolution
        if Path(c).is_symlink() and Path(c).resolve().exists():
            return c
    raise FileNotFoundError(
        "libstorm dylib not found. Install via `brew install stormlib` on macOS "
        "or build StormLib from source on Linux."
    )


_lib = None


def _stormlib():
    global _lib
    if _lib is None:
        _lib = ctypes.CDLL(_find_stormlib())

        # BOOL SFileCreateArchive(char* szMpqName, DWORD dwFlags, DWORD dwMaxFileCount, HANDLE* phMpq)
        _lib.SFileCreateArchive.argtypes = [c_char_p, c_uint, c_uint, POINTER(c_void_p)]
        _lib.SFileCreateArchive.restype = c_int

        # BOOL SFileAddFileEx(HANDLE hMpq, char* szFileName, char* szArchivedName, DWORD dwFlags, DWORD dwCompression, DWORD dwCompressionNext)
        _lib.SFileAddFileEx.argtypes = [c_void_p, c_char_p, c_char_p, c_uint, c_uint, c_uint]
        _lib.SFileAddFileEx.restype = c_int

        # BOOL SFileCloseArchive(HANDLE hMpq)
        _lib.SFileCloseArchive.argtypes = [c_void_p]
        _lib.SFileCloseArchive.restype = c_int

        # DWORD SErrGetLastError(void) — StormLib's cross-platform last-error.
        # (GetLastError is Win32-only and absent from the macOS/Linux dylib.)
        _lib.SErrGetLastError.argtypes = []
        _lib.SErrGetLastError.restype = c_uint
    return _lib


def pack_mpq(mpq_path: Path, files: Dict[str, bytes]) -> None:
    """Create or overwrite `mpq_path` containing the given internal-path -> bytes mapping.

    Keys use backslash separators (MPQ convention): e.g. "DBFilesClient\\ItemSet.dbc".
    """
    lib = _stormlib()
    mpq_path.parent.mkdir(parents=True, exist_ok=True)
    if mpq_path.exists():
        mpq_path.unlink()

    handle = c_void_p()
    ok = lib.SFileCreateArchive(
        str(mpq_path).encode("utf-8"),
        MPQ_CREATE_ARCHIVE_V1,
        max(len(files) * 4, 16),  # max_files (with headroom for hash table)
        byref(handle),
    )
    if not ok:
        err = lib.SErrGetLastError()
        raise RuntimeError(f"SFileCreateArchive failed (error code {err}) for {mpq_path}")

    try:
        # StormLib's SFileAddFileEx reads from a real file on disk, so we
        # write each payload to a tempfile and add by path.
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            for internal_path, data in files.items():
                # Translate "DBFilesClient\\ItemSet.dbc" -> tdp/DBFilesClient/ItemSet.dbc
                local = tdp / internal_path.replace("\\", "/")
                local.parent.mkdir(parents=True, exist_ok=True)
                local.write_bytes(data)

                ok = lib.SFileAddFileEx(
                    handle,
                    str(local).encode("utf-8"),
                    internal_path.encode("utf-8"),  # archive uses backslashes; StormLib accepts as-is
                    MPQ_FILE_COMPRESS,
                    MPQ_COMPRESSION_ZLIB,
                    MPQ_COMPRESSION_ZLIB,
                )
                if not ok:
                    err = lib.SErrGetLastError()
                    raise RuntimeError(f"SFileAddFileEx({internal_path}) failed (error code {err})")
    finally:
        lib.SFileCloseArchive(handle)
