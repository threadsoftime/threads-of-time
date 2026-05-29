#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Stage B — compose the unified ToT client MPQ.

Pipeline:
  1. Discover modules/<mod>/client/MANIFEST.toml
  2. RangeRegistry collision check on declared [id_ranges]        (check A)
  3. Import + run each recipe's build(sources) -> {dbc: blob}
  4. assert_ids_in_ranges on every produced blob: row IDs must be inside the
     module's declared ranges -> rejects stock Blizzard rows               (check B / §10.3)
  5. merge_dbc_outputs -> {dbc: blob}  (one-producer-per-file)
  6. Substitute @TOT_VERSION@ in the Stage A AddOn output
  7. Resolve FrameXML overrides by priority
  8. Pack DBCs + AddOn + FrameXML into one MPQ via StormLib
  9. Print sha256

Usage:
  python pack-mpq.py --version 1.0.0 --out build/patch-ZZ-tot-1.0.0.MPQ
  python pack-mpq.py --check          # build twice, assert identical sha256
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import shutil
import sys
import tempfile
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent          # tot/client-patch
REPO = HERE.parents[1]                            # threads-of-time
MODULES = REPO / "modules"
LIB = HERE / "lib" / "dbc_compositor" / "src"
ADDON_BUILD = HERE / "build" / "tot-addon" / "ThreadsOfTime"

# Startup path verification — the f094b9f6 repo migration broke parents[N] math
# in sibling scripts; print + assert these resolve to real on-disk paths so a
# future migration fails loud here instead of silently composing an empty MPQ.
for _name, _p, _must_exist in [
    ("HERE", HERE, True),
    ("REPO", REPO, True),
    ("MODULES", MODULES, True),
    ("LIB", LIB, True),
    ("ADDON_BUILD.parent", ADDON_BUILD.parent, False),  # build/ is created by Stage A
]:
    _ok = _p.exists()
    print(f"[path] {_name:18s} exists={_ok!s:5s} {_p}", file=sys.stderr)
    if _must_exist and not _ok:
        raise SystemExit(
            f"pack-mpq.py path resolution broken: {_name} -> {_p} does not exist. "
            f"Check parents[N] math (likely a repo-layout migration regression)."
        )

sys.path.insert(0, str(LIB))
from dbc_compositor.manifest import load_manifest, RangeRegistry      # noqa: E402
from dbc_compositor.merge import merge_dbc_outputs                    # noqa: E402
from dbc_compositor.framexml import resolve_overrides                 # noqa: E402
from dbc_compositor.dbc_audit import assert_ids_in_ranges            # noqa: E402
from dbc_compositor.mpq_pack import pack_mpq                          # noqa: E402


def _import_recipe(manifest):
    """Import modules/<mod>/client/<entry>.py with dbc_compositor importable."""
    recipe_path = manifest.manifest_path.parent / f"{manifest.recipe_entry}.py"
    spec = importlib.util.spec_from_file_location(
        f"recipe_{manifest.mod.replace('-', '_')}", recipe_path
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def compose_dbcs() -> dict[str, bytes]:
    registry = RangeRegistry()
    outputs: dict[str, dict[str, bytes]] = {}
    for manifest_path in sorted(MODULES.glob("*/client/MANIFEST.toml")):
        m = load_manifest(manifest_path)
        for dbc_name, ranges in m.id_ranges.items():
            registry.add(m.mod, dbc_name, ranges)              # check A
        recipe = _import_recipe(m)
        produced = recipe.build(m.sources)
        for dbc_name, blob in produced.items():
            if dbc_name not in m.id_ranges:
                raise SystemExit(
                    f"{m.mod} recipe produced {dbc_name} but MANIFEST.toml "
                    f"declares no [id_ranges] for it"
                )
            assert_ids_in_ranges(dbc_name, blob, m.id_ranges[dbc_name], m.mod)  # check B / §10.3
        outputs[m.mod] = produced
    return merge_dbc_outputs(outputs)


def collect_framexml() -> dict[str, str]:
    entries = []
    for manifest_path in sorted(MODULES.glob("*/client/MANIFEST.toml")):
        with open(manifest_path, "rb") as f:
            data = tomllib.load(f)
        root = manifest_path.parent.parent
        mod = data["manifest"]["mod"]
        for ov in data.get("framexml_overrides", []):
            entries.append({
                "mod": mod,
                "src": str((root / ov["src"]).resolve()),
                "dest": ov["dest"],
                "priority": int(ov["priority"]),
            })
    return resolve_overrides(entries)


def _staged_addon(version: str, tmp: Path) -> Path:
    """Copy the Stage A AddOn output, substituting @TOT_VERSION@."""
    if not ADDON_BUILD.exists():
        raise SystemExit(
            f"Stage A output missing at {ADDON_BUILD}. Run compose-tot-addon.py first."
        )
    dst = tmp / "ThreadsOfTime"
    shutil.copytree(ADDON_BUILD, dst)
    for lua in dst.rglob("*.lua"):
        text = lua.read_text()
        if "@TOT_VERSION@" in text:
            lua.write_text(text.replace("@TOT_VERSION@", version))
    return dst


def build_mpq(version: str, out: Path) -> str:
    dbcs = compose_dbcs()
    framexml = collect_framexml()
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        addon = _staged_addon(version, tmp)

        files: dict[str, bytes] = {}
        for dbc_name, blob in dbcs.items():
            files[f"DBFilesClient\\{dbc_name}"] = blob
        for f in addon.rglob("*"):
            if f.is_file():
                rel = f.relative_to(tmp).as_posix().replace("/", "\\")
                files[f"Interface\\AddOns\\{rel}"] = f.read_bytes()
        for dest, src in framexml.items():
            files[dest.replace("/", "\\")] = Path(src).read_bytes()

        out.parent.mkdir(parents=True, exist_ok=True)
        pack_mpq(out, files)
    return hashlib.sha256(out.read_bytes()).hexdigest()


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Compose the ToT client MPQ (Stage B)")
    ap.add_argument("--version", default="0.0.0-dev")
    ap.add_argument("--out", type=Path, default=HERE / "build" / "patch-ZZ-tot-0.0.0-dev.MPQ")
    ap.add_argument("--check", action="store_true",
                    help="build twice, assert identical sha256, then exit")
    args = ap.parse_args(argv)

    if args.check:
        a = build_mpq(args.version, args.out)
        b = build_mpq(args.version, args.out.with_suffix(".check.MPQ"))
        if a != b:
            print(f"NON-DETERMINISTIC: {a} != {b}", file=sys.stderr)
            return 1
        print(f"DETERMINISTIC OK sha256={a[:16]}...")
        return 0

    sha = build_mpq(args.version, args.out)
    print(f"DONE: {args.out} ({args.out.stat().st_size} bytes, sha256={sha[:16]}...)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
