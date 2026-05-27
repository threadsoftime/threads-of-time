#!/usr/bin/env python3
"""
Pack mod-warforged client assets into patch-W.MPQ.

Reads from:
    build/mpq-staging/DBFilesClient/SpellItemEnchantment.dbc
    build/tot-addon/ThreadsOfTime/*.lua + ThreadsOfTime.toc
        (composed from per-mod contributions by tools/compose-tot-addon.py)

Writes:
    client/patch-W.MPQ   (MPQ v1, ZLIB-compressed entries)

Drop the output in your WoW 3.3.5a client's Data/ folder.

v1.0.3 (this build) ships the AddOn at Interface\AddOns\ThreadsOfTime\ —
warforged contributions live alongside future bracket-sets / other-mod
contributions under one unified addon. The previous Interface\AddOns\Warforged\
was scoped to a single mod; the unified shape matches the long-term ToT 1.0.0
release pipeline (see kb_a217a790 + kb_bb38d450).

Run order:
    1. tools/compose-tot-addon.py        — assembles ThreadsOfTime/
    2. modules/mod-warforged/tools/build-warforged-dbc.py — patches DBC
    3. modules/mod-warforged/tools/pack-mpq.py            — bundles into MPQ

Library decision (2026-05-24):
    - mpyq v0.2.5 is reader-only (no create-empty API). Rejected.
    - pyMPQ is unmaintained / archive-only on PyPI. Rejected.
    - StormLib 9.31 is installed locally via Homebrew at
      /opt/homebrew/lib/libstorm.dylib with header at
      /opt/homebrew/include/StormLib.h. This is the same library
      AzerothCore itself uses; we ctypes-wrap the C API. Selected.

This script is host-portable: it searches a few well-known paths for
libstorm and bails with a clear error if it can't find it. To override
the search, set WARFORGED_STORMLIB=/path/to/libstorm.dylib (or .so).
"""

from __future__ import annotations

import ctypes
import ctypes.util
import os
import sys
from pathlib import Path

# --- Paths -----------------------------------------------------------------

ROOT = Path(__file__).resolve().parent.parent  # modules/mod-warforged/
REPO_ROOT = ROOT.parent.parent  # azerothcore-heimdal/
STAGING = ROOT / "build" / "mpq-staging"
TOT_ADDON_DIR = REPO_ROOT / "build" / "tot-addon" / "ThreadsOfTime"
OUTPUT = ROOT / "client" / "patch-W.MPQ"


def _build_internal_paths() -> dict[Path, str]:
    """
    Build the source-path -> MPQ-internal-path map.

    DBC stays at DBFilesClient/ — the client's DBC loader merge-overrides
    that table (safe).

    Everything in build/tot-addon/ThreadsOfTime/ ships under
    Interface\\AddOns\\ThreadsOfTime\\ — composed at release-time from
    per-mod contributions (tools/compose-tot-addon.py). Putting Lua at
    Interface\\FrameXML\\ would REPLACE stock files and crash the client
    with "interface files corrupt" (v1.0.1 bug).
    """
    paths: dict[Path, str] = {
        STAGING / "DBFilesClient" / "SpellItemEnchantment.dbc":
            "DBFilesClient\\SpellItemEnchantment.dbc",
    }
    if not TOT_ADDON_DIR.exists():
        raise SystemExit(
            f"{TOT_ADDON_DIR} does not exist. Run tools/compose-tot-addon.py "
            "first to assemble the composed AddOn."
        )
    for src in sorted(TOT_ADDON_DIR.iterdir()):
        if src.is_file() and src.name != ".DS_Store":
            paths[src] = f"Interface\\AddOns\\ThreadsOfTime\\{src.name}"
    return paths


INTERNAL_PATHS: dict[Path, str] = _build_internal_paths()

# --- StormLib constants (mirrored from StormLib.h) -------------------------

MPQ_CREATE_LISTFILE     = 0x00100000
MPQ_CREATE_ATTRIBUTES   = 0x00200000
MPQ_CREATE_ARCHIVE_V1   = 0x00000000   # client is 3.3.5a -> MPQ v1

MPQ_FILE_COMPRESS       = 0x00000200
MPQ_FILE_REPLACEEXISTING = 0x80000000

MPQ_COMPRESSION_ZLIB    = 0x02

# --- StormLib loading ------------------------------------------------------

_CANDIDATE_LIB_PATHS = [
    os.environ.get("WARFORGED_STORMLIB"),
    "/opt/homebrew/lib/libstorm.dylib",
    "/usr/local/lib/libstorm.dylib",
    "/usr/lib/libstorm.so",
    "/usr/local/lib/libstorm.so",
    "/usr/lib/x86_64-linux-gnu/libstorm.so",
]


def _load_stormlib() -> ctypes.CDLL:
    """Locate and load libstorm. Raise with a clear hint if not found."""
    for cand in _CANDIDATE_LIB_PATHS:
        if cand and Path(cand).exists():
            return ctypes.CDLL(cand)
    # Last try: let the dynamic loader search.
    name = ctypes.util.find_library("storm") or ctypes.util.find_library("Storm")
    if name:
        return ctypes.CDLL(name)
    raise RuntimeError(
        "Could not locate libstorm. Install via:\n"
        "    macOS:  brew install stormlib\n"
        "    Debian: apt install libstorm-dev\n"
        "or set WARFORGED_STORMLIB=/path/to/libstorm.{dylib,so}"
    )


def _bind_storm(storm: ctypes.CDLL) -> None:
    """Set argtypes/restypes for the functions we use."""
    # bool SFileCreateArchive(const TCHAR* szMpqName, DWORD dwCreateFlags,
    #                         DWORD dwMaxFileCount, HANDLE* phMpq);
    storm.SFileCreateArchive.argtypes = [
        ctypes.c_char_p, ctypes.c_uint32, ctypes.c_uint32,
        ctypes.POINTER(ctypes.c_void_p),
    ]
    storm.SFileCreateArchive.restype = ctypes.c_bool

    # bool SFileAddFileEx(HANDLE hMpq, const TCHAR* szFileName,
    #                     const char* szArchivedName, DWORD dwFlags,
    #                     DWORD dwCompression, DWORD dwCompressionNext);
    storm.SFileAddFileEx.argtypes = [
        ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p,
        ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32,
    ]
    storm.SFileAddFileEx.restype = ctypes.c_bool

    # bool SFileCloseArchive(HANDLE hMpq);
    storm.SFileCloseArchive.argtypes = [ctypes.c_void_p]
    storm.SFileCloseArchive.restype = ctypes.c_bool

    # DWORD GetLastError(); (StormLib mirrors Win32 codes via its own setter)
    # ctypes provides ctypes.get_errno()/get_last_error() on Windows only;
    # StormLib exports its own SetLastError, so we read errno via SFileGetLastError
    # if it exists. Not all builds export it -- treat absence as best-effort.
    if hasattr(storm, "SFileGetLastError"):
        storm.SFileGetLastError.argtypes = []
        storm.SFileGetLastError.restype = ctypes.c_uint32


def _storm_err(storm: ctypes.CDLL) -> str:
    if hasattr(storm, "SFileGetLastError"):
        return f"StormLib error code 0x{storm.SFileGetLastError():08x}"
    return "(StormLib error code unavailable on this build)"


# --- Pack logic ------------------------------------------------------------

def _validate_inputs() -> None:
    missing = [str(p) for p in INTERNAL_PATHS if not p.exists()]
    if missing:
        raise SystemExit(
            "Missing input files:\n  - " + "\n  - ".join(missing) +
            "\n\nRun tools/build-warforged-dbc.py first to produce the staging "
            "DBC + WarforgedStatBumps.lua, then check the Lua overrides under "
            "data/lua/."
        )


def pack() -> Path:
    _validate_inputs()

    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    if OUTPUT.exists():
        # Trash-then-replace; never use rm. Project rule.
        trash = Path.home() / ".Trash" / f"patch-W.MPQ.{os.getpid()}.bak"
        OUTPUT.replace(trash)

    storm = _load_stormlib()
    _bind_storm(storm)

    handle = ctypes.c_void_p()
    create_flags = (
        MPQ_CREATE_ARCHIVE_V1
        | MPQ_CREATE_LISTFILE
        | MPQ_CREATE_ATTRIBUTES
    )
    # Hash-table sized for the small fixed file count plus the listfile/
    # attributes meta-entries. Power-of-two; 16 leaves headroom for v1.1.
    max_file_count = 16

    ok = storm.SFileCreateArchive(
        str(OUTPUT).encode("utf-8"),
        create_flags,
        max_file_count,
        ctypes.byref(handle),
    )
    if not ok:
        raise RuntimeError(f"SFileCreateArchive failed: {_storm_err(storm)}")

    try:
        for src, internal in INTERNAL_PATHS.items():
            flags = MPQ_FILE_COMPRESS | MPQ_FILE_REPLACEEXISTING
            ok = storm.SFileAddFileEx(
                handle,
                str(src).encode("utf-8"),
                internal.encode("utf-8"),
                flags,
                MPQ_COMPRESSION_ZLIB,
                MPQ_COMPRESSION_ZLIB,
            )
            if not ok:
                raise RuntimeError(
                    f"SFileAddFileEx failed for {src} -> {internal}: "
                    f"{_storm_err(storm)}"
                )
            # Source can live under modules/mod-warforged (staging DBC) OR
            # under build/tot-addon/ at REPO_ROOT (composed AddOn). Use the
            # REPO_ROOT-relative form for consistent print output.
            try:
                rel = src.relative_to(REPO_ROOT)
            except ValueError:
                rel = src
            print(f"  + {internal:<48s} <- {rel}")
    finally:
        storm.SFileCloseArchive(handle)

    size = OUTPUT.stat().st_size
    print(f"\nWrote {OUTPUT} ({size:,} bytes)")
    return OUTPUT


if __name__ == "__main__":
    try:
        pack()
    except SystemExit:
        raise
    except Exception as exc:
        print(f"pack-mpq.py: {exc}", file=sys.stderr)
        sys.exit(1)
