# MPQ Compositor (Plan 4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Stage B client-MPQ compositor — a generalized, multi-module DBC + AddOn + branding packer with fail-loud DBC ID-range collision detection — producing a single `patch-ZZ-tot-<version>.MPQ` for the WoW 3.3.5a client.

**Architecture:** Generalize the existing `dbc-patch-builder` Python package (mod-bracket-sets-specific) into a shared library `tot/client-patch/lib/dbc_compositor/` plus per-module "recipes" at `modules/<mod>/client/build_dbc.py`. A new `tot/client-patch/pack-mpq.py` discovers per-module `MANIFEST.toml` files, runs each recipe, collision-checks the declared DBC ID ranges and the produced rows, merges rows per DBC file, pulls in the already-shipped Stage A AddOn output plus a new GlueXML branding AddOn and any FrameXML overrides, and packs one MPQ via StormLib. A golden-file test keeps the bracket-sets DBC output byte-identical to the live `patch-ZZ.MPQ` so the shipped tier-set tooltip rewrite never regresses.

**Tech Stack:** Python 3.12, `tomllib` (stdlib), StormLib via `ctypes` (already wrapped in `mpq_pack.py`), pytest. Pure client-side tooling — no C++, no worldserver changes.

---

## Ground truth (verified on disk 2026-05-28, do not re-derive)

These facts were confirmed by inspection. The plan depends on them; if any is false at execution time, STOP and re-verify.

- **Spec:** `docs/superpowers/specs/2026-05-28-threads-of-time-1.0.0-mpq-compositor-design.md`.
- **Existing builder:** `tot/client-patch/dbc-patch-builder/` — package `dbc_patch_builder` under `src/`. Modules: `build.py` (94 LOC, mod-bracket-sets-specific orchestrator), `check.py` (106), `dbc_io.py` (66), `itemset_dbc.py` (124), `spell_dbc.py` (130), `mpq_pack.py` (45, StormLib ctypes wrapper), `bonus_map_parser.py` (62), `__init__.py`. Tests: `tests/test_bonus_map_parser.py`, `test_build_determinism.py`, `test_itemset_dbc.py`, `test_mpq_pack.py`, `test_spell_dbc.py` (21 cases total, all green). `pyproject.toml`: `requires-python = ">=3.12"`, `[tool.pytest.ini_options] testpaths=["tests"] pythonpath=["src"]`.
- **The 54 marker Spell.dbc override IDs** (from `modules/mod-bracket-sets/data/fixtures/bracket_set_descriptions.tsv`): `64752 64753 64754 64755 64756 64757 64758 64760 64761 64762 64763 64764 64765 64766 64767 64768 64769 64770 64771 64772 64773 64774 64775 64776 64785 64786 64787 64788 64789 64790 64791 64792 64793 64794 64795 64796 64801 64802 64803 64804 64805 64806 64807 64808 64818 64819 64820 64821 64822 64823 64824 64825 64826 64827`. Count = 54, min = 64752, max = 64827 (sparse within that range — NOT the spec §5.8 placeholder `49000-49999`).
- **ItemSet.dbc IDs:** 28 rows = fallback `90101` + 27 class+spec rows `90111..90193`. Range claim: `90100-90199`.
- **mod-warforged ships ZERO DBC rows.** `modules/mod-warforged/client/patch-W.MPQ` is a **0-byte placeholder**; there is **no `modules/mod-warforged/client/dbc/` directory**. `modules/mod-warforged/tools/pack-mpq.py` is a stopgap stub that references aspirational `SpellItemEnchantment.dbc` IDs `4080001-4080005` which **do not exist on disk**. mod-warforged's entire client contribution is AddOn Lua (`data/addon-contrib/*.lua`) already handled by Stage A. **Therefore Plan 4 does NOT author a warforged DBC recipe — it deletes the dead scaffolding** (Task 11). For 1.0.0 there is exactly one real DBC contributor (mod-bracket-sets); collision-overlap behavior is exercised with a synthetic fixture module (Task 3).
- **Stage A composer (shipped `82a6583`):** `tot/client-patch/compose-tot-addon.py` globs `modules/*/data/addon-contrib/manifest.toml`, writes `tot/client-patch/build/tot-addon/ThreadsOfTime/`. Uses `os` but imports it lazily inside `__main__` (a latent bug — `compose()` calls `os.environ` at line 152 but `import os` is only in the `__main__` block; Task 8 touches this file and will hoist the import).
- **Live regression target:** the shipped `patch-ZZ.MPQ` sha256 = `3142d2362430287717bfaab836c45e764ecd0f26c2a24f1820241ec628aa5d47` (`kb_99501ac5`). The golden file we lock against is the raw DBC content the bracket-sets recipe produces (captured in Task 3), not the MPQ sha (MPQ compression is not guaranteed cross-StormLib-version stable).
- **StormLib dependency:** `brew install stormlib` (macOS) provides `libstorm.dylib`. `mpq_pack.py` already locates it. Tasks that pack a real MPQ require it; pure-logic tasks do not.

**Conventions for every task:** Python files get `# SPDX-License-Identifier: GPL-2.0-or-later` as the first line. Commits use the trailer:
```
Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
```
Work happens in `/Users/tbrack/Documents/Projects/threads-of-time` on branch `dev`.

---

## File structure (target end state)

```
tot/client-patch/
├── compose-tot-addon.py            [MODIFY: glob branding dir; hoist `import os`]
├── pack-mpq.py                     [CREATE: Stage B orchestrator + CLI]
├── PRIORITIES.md                   [exists]
├── DBC-RANGES.md                   [CREATE]
├── lib/
│   └── dbc_compositor/
│       ├── pyproject.toml          [CREATE: package config + pytest]
│       └── src/dbc_compositor/
│           ├── __init__.py         [CREATE]
│           ├── dbc_io.py           [MOVE from dbc-patch-builder, unchanged]
│           ├── itemset_dbc.py      [MOVE, unchanged]
│           ├── spell_dbc.py        [MOVE, unchanged]
│           ├── bonus_map_parser.py [MOVE, unchanged]
│           ├── mpq_pack.py         [MOVE, unchanged]
│           ├── manifest.py         [CREATE: Manifest loader + RangeRegistry]
│           ├── merge.py            [CREATE: per-DBC row union/sort/dedup]
│           └── framexml.py         [CREATE: priority-based override resolver]
│       └── tests/
│           ├── test_dbc_io.py            [MOVE subset]
│           ├── test_itemset_dbc.py       [MOVE]
│           ├── test_spell_dbc.py         [MOVE]
│           ├── test_bonus_map_parser.py  [MOVE]
│           ├── test_mpq_pack.py          [MOVE]
│           ├── test_manifest.py          [CREATE]
│           ├── test_merge.py             [CREATE]
│           └── test_framexml.py          [CREATE]
├── branding/
│   ├── addon-contrib/
│   │   ├── manifest.toml           [CREATE]
│   │   └── ToTBranding.lua         [CREATE]
│   └── README.md                   [CREATE]
└── tests/
    ├── conftest.py                 [CREATE: path bootstrap]
    ├── fixtures/
    │   └── fakemod/                [CREATE: synthetic 2nd DBC contributor]
    │       └── client/
    │           ├── MANIFEST.toml
    │           └── build_dbc.py
    ├── test_collision_integration.py  [CREATE]
    ├── test_branding_compose.py       [CREATE]
    ├── test_pack_determinism.py       [CREATE]
    └── test_golden_bracket_sets.py    [CREATE: regression guard]

modules/mod-bracket-sets/client/
├── MANIFEST.toml                   [CREATE]
├── build_dbc.py                    [CREATE: recipe from old build.py]
├── baseline/spell_dbc_baseline.tsv [MOVE from dbc-patch-builder/baseline/]
├── golden/itemset.dbc.golden       [CREATE: captured byte-exact blob]
├── golden/spell.dbc.golden         [CREATE: captured byte-exact blob]
└── tests/test_build_dbc.py         [CREATE: from old test_build_determinism.py]

modules/mod-warforged/
├── client/patch-W.MPQ              [DELETE: 0-byte dead placeholder]
└── tools/pack-mpq.py               [DELETE: dead stopgap stub]

docs/player-install.md              [CREATE]
tot/release/build-mpq.sh            [CREATE: CI hook]
MPQ_COMPOSITOR_STATE.md             [CREATE: end-state doc]
```

---

## Task 1: Scaffold the shared library, move generic modules unchanged

**Goal:** Stand up `lib/dbc_compositor/` as an installable package and relocate the five truly-generic modules (`dbc_io`, `itemset_dbc`, `spell_dbc`, `bonus_map_parser`, `mpq_pack`) and their tests verbatim. Prove the moved tests still pass. No logic changes.

**Files:**
- Create: `tot/client-patch/lib/dbc_compositor/pyproject.toml`
- Create: `tot/client-patch/lib/dbc_compositor/src/dbc_compositor/__init__.py`
- Move: 5 modules from `tot/client-patch/dbc-patch-builder/src/dbc_patch_builder/` → `tot/client-patch/lib/dbc_compositor/src/dbc_compositor/`
- Move: 5 test files from `dbc-patch-builder/tests/` → `lib/dbc_compositor/tests/`

- [ ] **Step 1: Create the package directory + pyproject**

Create `tot/client-patch/lib/dbc_compositor/pyproject.toml`:

```toml
[project]
name = "dbc-compositor"
version = "1.0.0"
requires-python = ">=3.12"

[tool.setuptools]
package-dir = {"" = "src"}

[tool.setuptools.packages.find]
where = ["src"]

[build-system]
requires = ["setuptools>=61"]

[tool.pytest.ini_options]
testpaths = ["tests"]
pythonpath = ["src"]
```

- [ ] **Step 2: Move the five generic modules via git mv**

Run:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/tot/client-patch
mkdir -p lib/dbc_compositor/src/dbc_compositor lib/dbc_compositor/tests
SRC=dbc-patch-builder/src/dbc_patch_builder
DST=lib/dbc_compositor/src/dbc_compositor
git mv $SRC/dbc_io.py          $DST/dbc_io.py
git mv $SRC/itemset_dbc.py     $DST/itemset_dbc.py
git mv $SRC/spell_dbc.py       $DST/spell_dbc.py
git mv $SRC/bonus_map_parser.py $DST/bonus_map_parser.py
git mv $SRC/mpq_pack.py        $DST/mpq_pack.py
```

These five modules import each other only via relative imports (`from .dbc_io import ...`), so moving them as a group keeps imports valid. No edits needed.

- [ ] **Step 3: Create the package __init__**

Create `tot/client-patch/lib/dbc_compositor/src/dbc_compositor/__init__.py`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
"""dbc_compositor — shared library for composing ToT client DBC patches.

Generic, mod-agnostic building blocks:
  - dbc_io          : DBC binary read/write primitives
  - itemset_dbc     : ItemSet.dbc row encoder
  - spell_dbc       : Spell.dbc override encoder
  - bonus_map_parser: parser for bracket_set_bonus_map seed SQL
  - mpq_pack        : StormLib MPQ packer (ctypes)
  - manifest        : MANIFEST.toml loader + RangeRegistry (Task 2)
  - merge           : per-DBC row union/sort/dedup (Task 4)
  - framexml        : priority-based FrameXML override resolver (Task 5)

Per-module composition lives in modules/<mod>/client/build_dbc.py recipes,
which import from this library.
"""
```

- [ ] **Step 4: Move the five test files**

Run:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/tot/client-patch
git mv dbc-patch-builder/tests/test_dbc_io.py          lib/dbc_compositor/tests/ 2>/dev/null || true
git mv dbc-patch-builder/tests/test_itemset_dbc.py     lib/dbc_compositor/tests/
git mv dbc-patch-builder/tests/test_spell_dbc.py       lib/dbc_compositor/tests/
git mv dbc-patch-builder/tests/test_bonus_map_parser.py lib/dbc_compositor/tests/
git mv dbc-patch-builder/tests/test_mpq_pack.py        lib/dbc_compositor/tests/
```
(The `test_dbc_io.py` move is `|| true` because that test may not exist as a standalone file; dbc_io may be covered inside other tests. Do not fail the task if it is absent.)

- [ ] **Step 5: Fix import paths in the moved tests**

The moved tests import `from dbc_patch_builder.X import ...`. Update to `from dbc_compositor.X import ...`.

Run:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/tot/client-patch/lib/dbc_compositor/tests
grep -rl 'dbc_patch_builder' . | xargs sed -i '' 's/dbc_patch_builder/dbc_compositor/g'
```
(macOS `sed -i ''`; on Linux use `sed -i`.)

- [ ] **Step 6: Run the moved tests, verify green**

Run:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/tot/client-patch/lib/dbc_compositor
python -m pytest -v
```
Expected: all moved cases PASS (the four non-mpq test files run anywhere; `test_mpq_pack.py` requires StormLib — if `libstorm` is absent it should skip/xfail, not error. If it hard-errors on missing StormLib, that is pre-existing behavior; note it and continue).

- [ ] **Step 7: Commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git add -A tot/client-patch/lib tot/client-patch/dbc-patch-builder
git commit -m "refactor(client-patch): extract dbc_compositor shared library

Move generic DBC/MPQ modules + tests from the mod-bracket-sets-specific
dbc-patch-builder into a shared dbc_compositor library. No logic changes;
imports rewired dbc_patch_builder -> dbc_compositor.

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Manifest loader + RangeRegistry (collision check A)

**Goal:** A `manifest.py` that loads `modules/<mod>/client/MANIFEST.toml`, resolves `[sources]` to absolute paths, and a `RangeRegistry` that fails loud on declared-range overlap across modules.

**Files:**
- Create: `tot/client-patch/lib/dbc_compositor/src/dbc_compositor/manifest.py`
- Test: `tot/client-patch/lib/dbc_compositor/tests/test_manifest.py`

- [ ] **Step 1: Write the failing test**

Create `tot/client-patch/lib/dbc_compositor/tests/test_manifest.py`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
import textwrap
from pathlib import Path

import pytest

from dbc_compositor.manifest import (
    Manifest,
    RangeRegistry,
    CollisionError,
    load_manifest,
)


def _write(tmp_path: Path, body: str, mod_root_files=None) -> Path:
    client = tmp_path / "client"
    client.mkdir(parents=True)
    (client / "MANIFEST.toml").write_text(textwrap.dedent(body))
    for rel in (mod_root_files or []):
        f = tmp_path / rel
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text("x")
    return client / "MANIFEST.toml"


def test_load_manifest_parses_fields_and_resolves_sources(tmp_path):
    mpath = _write(
        tmp_path,
        """
        [manifest]
        mod = "mod-example"
        [sources]
        seed = "data/seed.sql"
        [recipe]
        entry = "build_dbc"
        [id_ranges]
        "ItemSet.dbc" = [{ min = 90100, max = 90199 }]
        """,
        mod_root_files=["data/seed.sql"],
    )
    m = load_manifest(mpath)
    assert m.mod == "mod-example"
    assert m.recipe_entry == "build_dbc"
    assert m.sources["seed"] == (tmp_path / "data/seed.sql").resolve()
    assert m.id_ranges["ItemSet.dbc"] == [(90100, 90199)]


def test_load_manifest_missing_source_file_raises(tmp_path):
    mpath = _write(
        tmp_path,
        """
        [manifest]
        mod = "mod-example"
        [sources]
        seed = "data/missing.sql"
        [recipe]
        entry = "build_dbc"
        [id_ranges]
        "ItemSet.dbc" = [{ min = 1, max = 2 }]
        """,
    )
    with pytest.raises(FileNotFoundError, match="missing.sql"):
        load_manifest(mpath)


def test_range_registry_accepts_disjoint():
    reg = RangeRegistry()
    reg.add("mod-a", "Spell.dbc", [(64752, 64827)])
    reg.add("mod-b", "Spell.dbc", [(99000, 99099)])  # no raise


def test_range_registry_rejects_overlap():
    reg = RangeRegistry()
    reg.add("mod-a", "Spell.dbc", [(64752, 64818)])
    with pytest.raises(CollisionError, match="overlap"):
        reg.add("mod-b", "Spell.dbc", [(64800, 99099)])


def test_range_registry_overlap_message_names_both_modules():
    reg = RangeRegistry()
    reg.add("mod-a", "Spell.dbc", [(10, 20)])
    with pytest.raises(CollisionError) as exc:
        reg.add("mod-b", "Spell.dbc", [(15, 25)])
    msg = str(exc.value)
    assert "mod-a" in msg and "mod-b" in msg and "Spell.dbc" in msg


def test_range_registry_same_file_different_dbc_no_collision():
    reg = RangeRegistry()
    reg.add("mod-a", "Spell.dbc", [(10, 20)])
    reg.add("mod-a", "ItemSet.dbc", [(10, 20)])  # different file → fine
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd tot/client-patch/lib/dbc_compositor && python -m pytest tests/test_manifest.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'dbc_compositor.manifest'`.

- [ ] **Step 3: Write minimal implementation**

Create `tot/client-patch/lib/dbc_compositor/src/dbc_compositor/manifest.py`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
"""MANIFEST.toml loader + RangeRegistry (DBC ID-range collision check A).

A module declares its client DBC contribution in modules/<mod>/client/MANIFEST.toml:

    [manifest]
    mod = "mod-bracket-sets"
    [sources]                       # paths relative to the MODULE ROOT (client/..)
    bonus_map = "data/sql/world/....sql"
    [recipe]
    entry = "build_dbc"             # importable module next to MANIFEST.toml
    [id_ranges]                     # per-DBC-file inclusive ranges
    "ItemSet.dbc" = [{ min = 90100, max = 90199 }]

RangeRegistry ingests every module's [id_ranges] and raises CollisionError on
any overlap for the same DBC filename.
"""
from __future__ import annotations

import tomllib
from dataclasses import dataclass, field
from pathlib import Path


class CollisionError(Exception):
    """Raised when two modules claim overlapping DBC ID ranges."""


@dataclass
class Manifest:
    mod: str
    recipe_entry: str
    sources: dict[str, Path]
    id_ranges: dict[str, list[tuple[int, int]]]
    manifest_path: Path
    module_root: Path  # parent of client/


def load_manifest(manifest_path: Path) -> Manifest:
    manifest_path = Path(manifest_path).resolve()
    with open(manifest_path, "rb") as f:
        data = tomllib.load(f)

    mod = data["manifest"]["mod"]
    recipe_entry = data.get("recipe", {}).get("entry", "build_dbc")

    # client/MANIFEST.toml -> module root is two levels up
    module_root = manifest_path.parent.parent

    sources: dict[str, Path] = {}
    for key, rel in data.get("sources", {}).items():
        resolved = (module_root / rel).resolve()
        if not resolved.exists():
            raise FileNotFoundError(
                f"{manifest_path}: [sources].{key} -> {rel} does not exist "
                f"(resolved {resolved})"
            )
        sources[key] = resolved

    id_ranges: dict[str, list[tuple[int, int]]] = {}
    for dbc_name, ranges in data.get("id_ranges", {}).items():
        id_ranges[dbc_name] = [(int(r["min"]), int(r["max"])) for r in ranges]

    return Manifest(
        mod=mod,
        recipe_entry=recipe_entry,
        sources=sources,
        id_ranges=id_ranges,
        manifest_path=manifest_path,
        module_root=module_root,
    )


@dataclass
class RangeRegistry:
    # dbc_name -> list of (mod, min, max)
    _claims: dict[str, list[tuple[str, int, int]]] = field(default_factory=dict)

    def add(self, mod: str, dbc_name: str, ranges: list[tuple[int, int]]) -> None:
        existing = self._claims.setdefault(dbc_name, [])
        for (lo, hi) in ranges:
            for (other_mod, olo, ohi) in existing:
                if lo <= ohi and olo <= hi:  # interval overlap
                    overlap_lo, overlap_hi = max(lo, olo), min(hi, ohi)
                    raise CollisionError(
                        f"DBC ID-range collision in {dbc_name}\n"
                        f"  {other_mod} claims [{olo}, {ohi}]\n"
                        f"  {mod} claims [{lo}, {hi}]\n"
                        f"  overlap: [{overlap_lo}, {overlap_hi}]\n"
                        f"  Resolve by editing [id_ranges] in one module's "
                        f"client/MANIFEST.toml "
                        f"(allocation map: tot/client-patch/DBC-RANGES.md)"
                    )
            existing.append((mod, lo, hi))
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd tot/client-patch/lib/dbc_compositor && python -m pytest tests/test_manifest.py -v`
Expected: 6 PASS.

- [ ] **Step 5: Commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git add tot/client-patch/lib/dbc_compositor/src/dbc_compositor/manifest.py tot/client-patch/lib/dbc_compositor/tests/test_manifest.py
git commit -m "feat(client-patch): manifest loader + RangeRegistry collision check

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: mod-bracket-sets recipe + MANIFEST + golden capture

**Goal:** Port the mod-bracket-sets composition logic out of the old `build.py` into a recipe at `modules/mod-bracket-sets/client/build_dbc.py` exposing `build(sources) -> {dbc_name: blob}`, move the baseline TSV under the module, write the manifest with the REAL 54 spell IDs, and capture golden DBC blobs from the OLD builder before deleting it.

**Files:**
- Create: `modules/mod-bracket-sets/client/build_dbc.py`
- Create: `modules/mod-bracket-sets/client/MANIFEST.toml`
- Move: `tot/client-patch/dbc-patch-builder/baseline/spell_dbc_baseline.tsv` → `modules/mod-bracket-sets/client/baseline/spell_dbc_baseline.tsv`
- Create: `modules/mod-bracket-sets/client/golden/itemset.dbc.golden`, `spell.dbc.golden`
- Create: `modules/mod-bracket-sets/client/tests/test_build_dbc.py`

- [ ] **Step 1: Capture golden DBC blobs from the OLD builder FIRST**

Before touching anything, run the existing builder and extract the raw DBC blobs it produces. This is the byte-exact regression target.

Create a throwaway capture script and run it:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/tot/client-patch/dbc-patch-builder
mkdir -p /tmp/tot-golden
python - <<'PY'
import sys; sys.path.insert(0, "src")
from pathlib import Path
from dbc_patch_builder import build as B
# Re-run the exact compose path build.main() uses, but capture blobs.
bonus_rows = B.parse_bonus_map_seed(B.BONUS_SEED)
itemset_map = B.parse_itemset_map_seed(B.ITEMSET_MAP_SEED)
descriptions = B.load_descriptions(B.DESCRIPTIONS)
baseline = B.load_baseline(B.BASELINE)
from dbc_patch_builder.itemset_dbc import build_itemset_dbc, compose_27_class_spec_rows, fallback_row
from dbc_patch_builder.spell_dbc import SpellOverride, build_spell_dbc_overrides
itemset_rows = [fallback_row()] + compose_27_class_spec_rows(bonus_rows, itemset_map, class_items=B.CLASS_ITEMS)
itemset_blob = build_itemset_dbc(itemset_rows)
name_by_spell = {r.spell_id: r.display_name for r in bonus_rows}
overrides = []
for sid in sorted(descriptions):
    flavor, mech = descriptions[sid]
    overrides.append(SpellOverride(spell_id=sid, name=name_by_spell[sid],
        description=f"{flavor}\r{mech}\r{B.BRACKET_WINDOW_LINE}"))
spell_blob = build_spell_dbc_overrides(baseline, overrides)
Path("/tmp/tot-golden/itemset.dbc.golden").write_bytes(itemset_blob)
Path("/tmp/tot-golden/spell.dbc.golden").write_bytes(spell_blob)
import hashlib
print("itemset", len(itemset_blob), hashlib.sha256(itemset_blob).hexdigest()[:16])
print("spell  ", len(spell_blob), hashlib.sha256(spell_blob).hexdigest()[:16])
PY
```
Expected: prints two non-zero sizes + sha prefixes. RECORD these two sha prefixes in the commit message of this task — they are the regression anchor.

- [ ] **Step 2: Move baseline + golden into the module**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
mkdir -p modules/mod-bracket-sets/client/baseline modules/mod-bracket-sets/client/golden
git mv tot/client-patch/dbc-patch-builder/baseline/spell_dbc_baseline.tsv modules/mod-bracket-sets/client/baseline/spell_dbc_baseline.tsv
cp /tmp/tot-golden/itemset.dbc.golden modules/mod-bracket-sets/client/golden/itemset.dbc.golden
cp /tmp/tot-golden/spell.dbc.golden   modules/mod-bracket-sets/client/golden/spell.dbc.golden
```

- [ ] **Step 3: Write the recipe**

Create `modules/mod-bracket-sets/client/build_dbc.py`. This is the old `build.py` compose logic, restructured to the `build(sources)` contract and importing `dbc_compositor`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
"""mod-bracket-sets client DBC recipe.

Exposes build(sources) -> {dbc_filename: bytes} for the Stage B compositor.
Composes ItemSet.dbc (28 rows: fallback 90101 + 27 class+spec 90111..90193)
and Spell.dbc (54 marker-spell name/description overrides) from this module's
SQL/TSV sources, preserving every non-text Spell field from the baseline.

Logic ported verbatim from the original tot/client-patch/dbc-patch-builder
build.py (kb_99501ac5). The golden/ blobs lock the output byte-for-byte.
"""
from __future__ import annotations

import csv
import re
from pathlib import Path

from dbc_compositor.bonus_map_parser import parse_bonus_map_seed
from dbc_compositor.itemset_dbc import (
    build_itemset_dbc,
    compose_27_class_spec_rows,
    fallback_row,
)
from dbc_compositor.spell_dbc import (
    SpellOverride,
    load_baseline,
    build_spell_dbc_overrides,
)

BRACKET_WINDOW_LINE = "Active while in Bracket 1 (L25-34)"

# Member items per armor type, queried from item_template WHERE itemset=90101.
# Ported verbatim from the original build.py (kb_99501ac5). Do not edit without
# re-capturing golden blobs.
_ITEMS_MISC = [90004, 90005, 90009, 90020, 90025, 90037, 90039, 90044, 90048]
_ITEMS_CLOTH = [90007, 90038, 90040, 90041, 90042, 90043, 90046, 90051, 90201, 90205]
_ITEMS_LEATHER = [90003, 90010, 90012, 90024, 90027, 90033, 90049, 90203]
_ITEMS_MAIL = [90000, 90002, 90021, 90023, 90036, 90047, 90202]
_ITEMS_PLATE = [90200, 90204, 90206]

CLASS_ITEMS = {
    1: _ITEMS_PLATE + _ITEMS_MISC,
    2: _ITEMS_PLATE + _ITEMS_MISC,
    3: _ITEMS_MAIL + _ITEMS_MISC,
    4: _ITEMS_LEATHER + _ITEMS_MISC,
    5: _ITEMS_CLOTH + _ITEMS_MISC,
    6: _ITEMS_PLATE + _ITEMS_MISC,
    7: _ITEMS_MAIL + _ITEMS_MISC,
    8: _ITEMS_CLOTH + _ITEMS_MISC,
    9: _ITEMS_CLOTH + _ITEMS_MISC,
    11: _ITEMS_LEATHER + _ITEMS_MISC,
}


def _load_descriptions(tsv_path: Path) -> dict[int, tuple[str, str]]:
    out: dict[int, tuple[str, str]] = {}
    with tsv_path.open() as f:
        for row in csv.reader(f, delimiter="\t"):
            if not row or row[0].startswith("#") or row[0] == "spell_id":
                continue
            out[int(row[0])] = (row[1], row[2])
    return out


def _parse_itemset_map_seed(sql_path: Path) -> dict[tuple[int, int], int]:
    out: dict[tuple[int, int], int] = {}
    text = sql_path.read_text()
    for m in re.finditer(r"\(\s*(\d+)\s*,\s*(\d+)\s*,\s*1\s*,\s*(\d+)\s*\)", text):
        out[(int(m.group(1)), int(m.group(2)))] = int(m.group(3))
    return out


def build(sources: dict[str, Path]) -> dict[str, bytes]:
    bonus_rows = parse_bonus_map_seed(sources["bonus_map"])
    assert len(bonus_rows) == 54, f"expected 54 bonus rows, got {len(bonus_rows)}"

    itemset_map = _parse_itemset_map_seed(sources["itemset_map"])
    assert len(itemset_map) == 27, f"expected 27 itemset rows, got {len(itemset_map)}"

    descriptions = _load_descriptions(sources["descriptions"])
    assert len(descriptions) == 54, f"expected 54 descriptions, got {len(descriptions)}"

    baseline = load_baseline(sources["spell_baseline"])

    itemset_rows = [fallback_row()] + compose_27_class_spec_rows(
        bonus_rows, itemset_map, class_items=CLASS_ITEMS
    )
    itemset_blob = build_itemset_dbc(itemset_rows)

    name_by_spell = {r.spell_id: r.display_name for r in bonus_rows}
    overrides = []
    for spell_id in sorted(descriptions):
        flavor, mechanic = descriptions[spell_id]
        overrides.append(
            SpellOverride(
                spell_id=spell_id,
                name=name_by_spell[spell_id],
                description=f"{flavor}\r{mechanic}\r{BRACKET_WINDOW_LINE}",
            )
        )
    spell_blob = build_spell_dbc_overrides(baseline, overrides)

    return {"ItemSet.dbc": itemset_blob, "Spell.dbc": spell_blob}
```

- [ ] **Step 4: Write the MANIFEST with the real 54 spell IDs**

Create `modules/mod-bracket-sets/client/MANIFEST.toml`. The 54 IDs span 64752–64827 but are sparse; express as the tight enclosing range (collision check A is range-based, check B asserts each produced row is inside it — both satisfied by the enclosing range):

```toml
# mod-bracket-sets client DBC contribution.
# Sources are relative to the module root (the directory ABOVE client/).
[manifest]
mod = "mod-bracket-sets"

[sources]
bonus_map      = "data/sql/world/2026_05_13_01_bracket_set_bonus_map_seed.sql"
itemset_map    = "data/sql/world/2026_05_23_07_bracket_set_itemset_map_seed.sql"
descriptions   = "data/fixtures/bracket_set_descriptions.tsv"
spell_baseline = "client/baseline/spell_dbc_baseline.tsv"

[recipe]
entry = "build_dbc"

# DBC ID ranges OWNED by this module. Collision detection fails loud if any
# other module's MANIFEST claims an overlapping range for the same file.
#   ItemSet.dbc: fallback 90101 + 27 class+spec rows 90111..90193
#   Spell.dbc:   54 marker-spell overrides, sparse within 64752..64827
[id_ranges]
"ItemSet.dbc" = [{ min = 90100, max = 90199 }]
"Spell.dbc"   = [{ min = 64752, max = 64827 }]
```

- [ ] **Step 5: Write the recipe + golden test**

Create `modules/mod-bracket-sets/client/tests/test_build_dbc.py`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
import sys
from pathlib import Path

import pytest

# Make the recipe importable (it sits next to this tests/ dir's parent).
CLIENT_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(CLIENT_DIR))
# Make dbc_compositor importable.
LIB = CLIENT_DIR.parents[1] / "tot" / "client-patch" / "lib" / "dbc_compositor" / "src"
sys.path.insert(0, str(LIB))

import build_dbc  # noqa: E402
from dbc_compositor.manifest import load_manifest  # noqa: E402

MANIFEST = CLIENT_DIR / "MANIFEST.toml"
GOLDEN = CLIENT_DIR / "golden"


def _run():
    m = load_manifest(MANIFEST)
    return build_dbc.build(m.sources)


def test_build_returns_both_dbcs():
    out = _run()
    assert set(out) == {"ItemSet.dbc", "Spell.dbc"}


def test_itemset_blob_matches_golden():
    out = _run()
    assert out["ItemSet.dbc"] == (GOLDEN / "itemset.dbc.golden").read_bytes()


def test_spell_blob_matches_golden():
    out = _run()
    assert out["Spell.dbc"] == (GOLDEN / "spell.dbc.golden").read_bytes()


def test_determinism_two_runs_identical():
    assert _run() == _run()


def test_all_spell_rows_within_declared_range():
    """Check B precondition: every produced Spell row ID is inside 64752..64827."""
    m = load_manifest(MANIFEST)
    lo, hi = m.id_ranges["Spell.dbc"][0]
    # The recipe's source-of-truth IDs come from the descriptions TSV.
    import csv
    ids = []
    with m.sources["descriptions"].open() as f:
        for row in csv.reader(f, delimiter="\t"):
            if row and row[0].isdigit():
                ids.append(int(row[0]))
    assert len(ids) == 54
    assert all(lo <= i <= hi for i in ids)
```

- [ ] **Step 6: Run the recipe tests, verify green**

Run:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/modules/mod-bracket-sets/client
python -m pytest tests/ -v
```
Expected: 5 PASS. The golden tests prove the recipe reproduces the live DBC bytes exactly.

- [ ] **Step 7: Commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git add modules/mod-bracket-sets/client tot/client-patch/dbc-patch-builder
git commit -m "feat(mod-bracket-sets): client DBC recipe + MANIFEST + golden blobs

Port bracket-sets DBC composition from dbc-patch-builder/build.py into a
build_dbc.py recipe under modules/mod-bracket-sets/client/. Golden blobs
captured from the live builder lock ItemSet.dbc + Spell.dbc byte-for-byte
(itemset sha=<RECORD>, spell sha=<RECORD>). 54 Spell overrides 64752..64827.

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: DBC row-merge engine (collision checks B + dedup)

**Goal:** A `merge.py` that, given each module's manifest + the `{dbc_name: blob}` its recipe returned, validates (check B) every shipped row is inside the module's declared range, then unions per-DBC-file blobs across modules. Because each recipe returns a *complete DBC blob per file* (not individual rows), and 1.0.0 has one DBC contributor per file, "merge" for 1.0.0 is: assert no two modules produce the same DBC filename, and assert declared ranges already passed check A. Build the engine general enough for multi-contributor but keep it honest about the row-level granularity.

> **Design note for the implementer:** the recipes return whole encoded DBC blobs, not row lists, because the existing encoders (`build_itemset_dbc`, `build_spell_dbc_overrides`) operate on full row sets and emit a complete file. True row-level cross-module union of the SAME DBC file would require decoding/re-encoding and is unnecessary for 1.0.0 (no two modules write the same DBC file). The merge engine therefore enforces **one producer per DBC filename** and fails loud if two modules both emit e.g. `Spell.dbc`. The RangeRegistry (check A) remains the ID-overlap guard; this is the file-level guard. Document this boundary in the module docstring.

**Files:**
- Create: `tot/client-patch/lib/dbc_compositor/src/dbc_compositor/merge.py`
- Test: `tot/client-patch/lib/dbc_compositor/tests/test_merge.py`

- [ ] **Step 1: Write the failing test**

Create `tot/client-patch/lib/dbc_compositor/tests/test_merge.py`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
import pytest

from dbc_compositor.merge import merge_dbc_outputs, DuplicateProducerError


def test_single_producer_passes_through():
    outputs = {"mod-a": {"Spell.dbc": b"AAA", "ItemSet.dbc": b"BBB"}}
    merged = merge_dbc_outputs(outputs)
    assert merged == {"Spell.dbc": b"AAA", "ItemSet.dbc": b"BBB"}


def test_two_producers_different_files_merge():
    outputs = {
        "mod-a": {"Spell.dbc": b"AAA"},
        "mod-b": {"ItemSet.dbc": b"BBB"},
    }
    merged = merge_dbc_outputs(outputs)
    assert merged == {"Spell.dbc": b"AAA", "ItemSet.dbc": b"BBB"}


def test_two_producers_same_file_raises():
    outputs = {
        "mod-a": {"Spell.dbc": b"AAA"},
        "mod-b": {"Spell.dbc": b"CCC"},
    }
    with pytest.raises(DuplicateProducerError, match="Spell.dbc"):
        merge_dbc_outputs(outputs)


def test_duplicate_message_names_both_mods():
    outputs = {"mod-a": {"Spell.dbc": b"A"}, "mod-b": {"Spell.dbc": b"B"}}
    with pytest.raises(DuplicateProducerError) as exc:
        merge_dbc_outputs(outputs)
    assert "mod-a" in str(exc.value) and "mod-b" in str(exc.value)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd tot/client-patch/lib/dbc_compositor && python -m pytest tests/test_merge.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'dbc_compositor.merge'`.

- [ ] **Step 3: Write minimal implementation**

Create `tot/client-patch/lib/dbc_compositor/src/dbc_compositor/merge.py`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
"""Merge per-module DBC recipe outputs into one {dbc_name: blob} mapping.

Each recipe returns a COMPLETE encoded DBC blob per filename (the existing
encoders emit whole files, not row deltas). For 1.0.0 no two modules write
the same DBC file, so merging is file-level: one producer per DBC filename.
If two modules both emit the same DBC filename, that is a hard error — row-
level union of a single DBC file across modules is intentionally out of scope
(would require decode/re-encode; add when a real second producer needs it).

ID-range overlap across modules is guarded separately by RangeRegistry
(manifest.py, check A). This module is the file-level producer guard.
"""
from __future__ import annotations


class DuplicateProducerError(Exception):
    """Raised when two modules both produce the same DBC filename."""


def merge_dbc_outputs(outputs: dict[str, dict[str, bytes]]) -> dict[str, bytes]:
    """outputs: {mod_name: {dbc_filename: blob}} -> {dbc_filename: blob}."""
    merged: dict[str, bytes] = {}
    producer: dict[str, str] = {}
    for mod, files in outputs.items():
        for dbc_name, blob in files.items():
            if dbc_name in merged:
                raise DuplicateProducerError(
                    f"Two modules produce {dbc_name}: "
                    f"{producer[dbc_name]} and {mod}. "
                    f"Row-level merge of one DBC file across modules is not "
                    f"supported; give them disjoint DBC files or merge upstream."
                )
            merged[dbc_name] = blob
            producer[dbc_name] = mod
    return merged
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd tot/client-patch/lib/dbc_compositor && python -m pytest tests/test_merge.py -v`
Expected: 4 PASS.

- [ ] **Step 5: Commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git add tot/client-patch/lib/dbc_compositor/src/dbc_compositor/merge.py tot/client-patch/lib/dbc_compositor/tests/test_merge.py
git commit -m "feat(client-patch): DBC output merge engine (one-producer-per-file guard)

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: FrameXML override resolver (priority-based)

**Goal:** A `framexml.py` resolving `[framexml_overrides]` entries `{src, dest, priority}` across modules: higher priority wins on a shared `dest`; equal priority on the same `dest` is a hard error.

**Files:**
- Create: `tot/client-patch/lib/dbc_compositor/src/dbc_compositor/framexml.py`
- Test: `tot/client-patch/lib/dbc_compositor/tests/test_framexml.py`

- [ ] **Step 1: Write the failing test**

Create `tot/client-patch/lib/dbc_compositor/tests/test_framexml.py`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
import pytest

from dbc_compositor.framexml import resolve_overrides, FrameXmlConflictError


def test_no_overrides_returns_empty():
    assert resolve_overrides([]) == {}


def test_single_override():
    entries = [{"mod": "mod-a", "src": "/a/X.xml", "dest": "Interface/X.xml", "priority": 10}]
    out = resolve_overrides(entries)
    assert out == {"Interface/X.xml": "/a/X.xml"}


def test_higher_priority_wins_same_dest():
    entries = [
        {"mod": "mod-a", "src": "/a/X.xml", "dest": "Interface/X.xml", "priority": 10},
        {"mod": "mod-b", "src": "/b/X.xml", "dest": "Interface/X.xml", "priority": 20},
    ]
    out = resolve_overrides(entries)
    assert out["Interface/X.xml"] == "/b/X.xml"  # priority 20 wins


def test_equal_priority_same_dest_raises():
    entries = [
        {"mod": "mod-a", "src": "/a/X.xml", "dest": "Interface/X.xml", "priority": 10},
        {"mod": "mod-b", "src": "/b/X.xml", "dest": "Interface/X.xml", "priority": 10},
    ]
    with pytest.raises(FrameXmlConflictError, match="Interface/X.xml"):
        resolve_overrides(entries)


def test_different_dests_coexist():
    entries = [
        {"mod": "mod-a", "src": "/a/X.xml", "dest": "Interface/X.xml", "priority": 10},
        {"mod": "mod-b", "src": "/b/Y.xml", "dest": "Interface/Y.xml", "priority": 10},
    ]
    out = resolve_overrides(entries)
    assert len(out) == 2
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd tot/client-patch/lib/dbc_compositor && python -m pytest tests/test_framexml.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'dbc_compositor.framexml'`.

- [ ] **Step 3: Write minimal implementation**

Create `tot/client-patch/lib/dbc_compositor/src/dbc_compositor/framexml.py`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
"""Priority-based FrameXML override resolution.

A module may ship raw FrameXML/GlueXML file overrides via [framexml_overrides]
in its MANIFEST.toml:

    [[framexml_overrides]]
    src = "client/framexml/GlueParent.xml"   # relative to module root
    dest = "Interface/GlueXML/GlueParent.xml" # path inside the MPQ
    priority = 50

When two modules target the same dest, the higher priority wins. Equal
priority on the same dest is a hard error (never silent last-write-wins).
No module ships overrides today; this exists for future use and as the
branding fallback rail if GlueXML AddOns do not load on a client build.
"""
from __future__ import annotations


class FrameXmlConflictError(Exception):
    """Raised when two equal-priority overrides target the same dest."""


def resolve_overrides(entries: list[dict]) -> dict[str, str]:
    """entries: list of {mod, src, dest, priority} -> {dest: winning_src}."""
    best: dict[str, dict] = {}
    for e in entries:
        dest = e["dest"]
        cur = best.get(dest)
        if cur is None:
            best[dest] = e
        elif e["priority"] > cur["priority"]:
            best[dest] = e
        elif e["priority"] == cur["priority"]:
            raise FrameXmlConflictError(
                f"FrameXML override conflict on {dest}: "
                f"{cur['mod']} and {e['mod']} both claim priority "
                f"{e['priority']}. Give one a higher priority to resolve."
            )
        # else: keep the existing higher-priority winner
    return {dest: e["src"] for dest, e in best.items()}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd tot/client-patch/lib/dbc_compositor && python -m pytest tests/test_framexml.py -v`
Expected: 5 PASS.

- [ ] **Step 5: Commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git add tot/client-patch/lib/dbc_compositor/src/dbc_compositor/framexml.py tot/client-patch/lib/dbc_compositor/tests/test_framexml.py
git commit -m "feat(client-patch): priority-based FrameXML override resolver

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Branding overlay (GlueXML AddOn) + composer glob extension

**Goal:** Create the branding "module" (`tot/client-patch/branding/`) with a `ToTBranding.lua` login-screen watermark + §10.4 disclaimer, and extend `compose-tot-addon.py` to discover it. Templating of the version string happens in `pack-mpq.py` (Task 7); here `ToTBranding.lua` carries a `@TOT_VERSION@` token.

**Files:**
- Create: `tot/client-patch/branding/addon-contrib/manifest.toml`
- Create: `tot/client-patch/branding/addon-contrib/ToTBranding.lua`
- Create: `tot/client-patch/branding/README.md`
- Modify: `tot/client-patch/compose-tot-addon.py`
- Test: `tot/client-patch/tests/test_branding_compose.py`, `tot/client-patch/tests/conftest.py`

- [ ] **Step 1: Create the branding manifest + Lua + README**

Create `tot/client-patch/branding/addon-contrib/manifest.toml`:
```toml
# ToT login-screen branding. Composed through Stage A (compose-tot-addon.py).
# priority 5 = core band, loads before feature mods.
mod = "tot-branding"
priority = 5

files = ["ToTBranding.lua"]
```

Create `tot/client-patch/branding/addon-contrib/ToTBranding.lua`:
```lua
-- ToTBranding.lua — login-screen version watermark + mandatory fan-project
-- disclaimer (design §5.2, spec §10.4). @TOT_VERSION@ is substituted at
-- compose time by pack-mpq.py. AUTO-COMPOSED into the ThreadsOfTime AddOn.
--
-- PRE-FLIGHT (verify on the real 3.3.5a client before shipping):
--   1. The glue version FontString name. `VersionLabel` is the common 3.3.5a
--      name; if absent, this hook no-ops harmlessly. Confirm on the client at
--      ChromieCraft_3.3.5a/ and adjust the global if needed.
--   2. Whether glue-screen AddOns load on this build. If they do not, ship the
--      branding via the FrameXML-override rail instead (design §5.3).

ThreadsOfTime = ThreadsOfTime or {}
ThreadsOfTime.Branding = ThreadsOfTime.Branding or {}

local TOT_VERSION = "@TOT_VERSION@"
local DISCLAIMER = "Unofficial non-commercial fan project. Not affiliated with"
    .. " Blizzard Entertainment. WoW and Wrath of the Lich King are trademarks"
    .. " of Blizzard Entertainment."

local function applyBranding()
    if _G.VersionLabel and _G.VersionLabel.SetText then
        _G.VersionLabel:SetText(
            "Threads of Time " .. TOT_VERSION .. "\n|cff888888" .. DISCLAIMER .. "|r")
    end
end

-- Glue AddOns run on the login screen; apply immediately and on show.
applyBranding()
if _G.GlueParent and _G.GlueParent.HookScript then
    _G.GlueParent:HookScript("OnShow", applyBranding)
end
```

Create `tot/client-patch/branding/README.md`:
```markdown
# ToT client branding overlay

`addon-contrib/ToTBranding.lua` sets the login-screen version watermark and the
mandatory fan-project disclaimer (spec §10.4). It is composed into the unified
`ThreadsOfTime` AddOn by `compose-tot-addon.py` (Stage A) and packed into the
client MPQ by `pack-mpq.py` (Stage B), which substitutes `@TOT_VERSION@`.

This is a synthetic "module" — not an AzerothCore module — so it lives under
`tot/client-patch/` rather than `modules/`. Priority 5 (core band) loads it
before feature-mod contributions.

Disclaimer copy + placement tone is owned by game-design-architect; the minimum
bar is that the fan-project disclaimer is visible on the login screen.

Splash imagery is out of scope for 1.0.0 (text-only branding).
```

- [ ] **Step 2: Write the failing test**

Create `tot/client-patch/tests/conftest.py`:
```python
# SPDX-License-Identifier: GPL-2.0-or-later
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]  # threads-of-time/
LIB = ROOT / "tot" / "client-patch" / "lib" / "dbc_compositor" / "src"
sys.path.insert(0, str(LIB))
```

Create `tot/client-patch/tests/test_branding_compose.py`:
```python
# SPDX-License-Identifier: GPL-2.0-or-later
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
COMPOSER = ROOT / "tot" / "client-patch" / "compose-tot-addon.py"
ADDON_OUT = ROOT / "tot" / "client-patch" / "build" / "tot-addon" / "ThreadsOfTime"


def test_composer_includes_branding():
    # Run the real composer; it must pick up tot/client-patch/branding/.
    subprocess.run([sys.executable, str(COMPOSER)], check=True, cwd=ROOT)
    composed = list(ADDON_OUT.glob("*ToTBranding*"))
    assert composed, f"branding not composed into {ADDON_OUT}"
    # priority 5 => filename prefix "05-"
    assert any(p.name.startswith("05-") for p in composed)


def test_branding_disclaimer_present():
    files = list(ADDON_OUT.glob("*ToTBranding*"))
    assert files
    text = files[0].read_text()
    assert "fan project" in text.lower()
    assert "@TOT_VERSION@" in text  # still tokenized at Stage A
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd /Users/tbrack/Documents/Projects/threads-of-time && python -m pytest tot/client-patch/tests/test_branding_compose.py -v`
Expected: FAIL — branding not composed (composer doesn't glob the branding dir yet).

- [ ] **Step 4: Extend the composer**

Modify `tot/client-patch/compose-tot-addon.py`. Two edits:

(a) Hoist `import os` to the top of the file (it's currently only inside `__main__`, but `compose()` uses `os.environ` at line ~152). Add `import os` to the import block near line 34 (`import shutil`).

(b) In `discover_contributions()`, after the `for manifest_path in MODULES.glob(...)` loop, also discover the branding manifest. Change the discovery to include the branding path. Replace the loop header:

```python
def discover_contributions() -> list[dict]:
    """Find every modules/<mod>/data/addon-contrib/manifest.toml + its files,
    plus the synthetic tot/client-patch/branding/addon-contrib/manifest.toml."""
    contribs = []
    manifest_paths = list(MODULES.glob("*/data/addon-contrib/manifest.toml"))
    branding = ROOT / "tot" / "client-patch" / "branding" / "addon-contrib" / "manifest.toml"
    if branding.exists():
        manifest_paths.append(branding)
    for manifest_path in manifest_paths:
        ...  # body unchanged
```

Note: `ROOT` in `compose-tot-addon.py` is `Path(__file__).resolve().parent.parent` (= `threads-of-time/tot`... verify). If `ROOT` points at the repo root the branding path is `ROOT/"tot"/...`; if it points at `tot/` adjust to `ROOT/"client-patch"/"branding"/...`. **Verify `ROOT` by reading the file's line 39 before editing** and use the correct relative path. The `MODULES` glob currently resolves correctly, so compute branding relative to the same base `MODULES.parent`.

Safest form (independent of ROOT semantics):
```python
    branding = MODULES.parent / "tot" / "client-patch" / "branding" / "addon-contrib" / "manifest.toml"
```
Only if `MODULES.parent` is the repo root. **Read line 39-41 first and pick the expression that resolves to the real path; confirm with a print before finalizing.**

- [ ] **Step 5: Run test to verify it passes**

Run: `cd /Users/tbrack/Documents/Projects/threads-of-time && python -m pytest tot/client-patch/tests/test_branding_compose.py -v`
Expected: 2 PASS.

- [ ] **Step 6: Commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git add tot/client-patch/branding tot/client-patch/compose-tot-addon.py tot/client-patch/tests/conftest.py tot/client-patch/tests/test_branding_compose.py
git commit -m "feat(client-patch): login-screen branding overlay + composer glob

Add tot/client-patch/branding/ (GlueXML AddOn: version watermark + §10.4
fan-project disclaimer) and extend compose-tot-addon.py to discover it.
@TOT_VERSION@ token substituted at pack time. Hoist import os to module top.

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: pack-mpq.py — the Stage B orchestrator

**Goal:** The CLI that ties everything together: discover manifests → check A (RangeRegistry) → run recipes → check B (row range containment via the descriptions/IDs) → merge → provenance audit → substitute `@TOT_VERSION@` in the composed AddOn → pack the MPQ (DBCs + AddOn + FrameXML overrides) → emit sha256. Plus `--check` (build twice, assert identical).

**Files:**
- Create: `tot/client-patch/pack-mpq.py`
- Test: covered by Tasks 8 (golden) + integration (Task 9). A focused unit test for arg handling goes here.

- [ ] **Step 1: Write pack-mpq.py**

Create `tot/client-patch/pack-mpq.py`:

```python
#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Stage B — compose the unified ToT client MPQ.

Pipeline:
  1. Discover modules/<mod>/client/MANIFEST.toml
  2. RangeRegistry collision check on declared [id_ranges]   (check A)
  3. Import + run each recipe's build(sources) -> {dbc: blob}
  4. Assert each produced DBC's declared range is non-empty   (check B precond)
  5. merge_dbc_outputs -> {dbc: blob}  (one-producer-per-file)
  6. Provenance: every packed DBC came from a recipe, never a .dbc on disk
  7. Substitute @TOT_VERSION@ in the Stage A AddOn output
  8. Resolve FrameXML overrides by priority
  9. Pack DBCs + AddOn + FrameXML into one MPQ via StormLib
 10. Print sha256

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
from pathlib import Path

HERE = Path(__file__).resolve().parent          # tot/client-patch
REPO = HERE.parents[1]                            # threads-of-time
MODULES = REPO / "modules"
LIB = HERE / "lib" / "dbc_compositor" / "src"
ADDON_BUILD = HERE / "build" / "tot-addon" / "ThreadsOfTime"

sys.path.insert(0, str(LIB))
from dbc_compositor.manifest import load_manifest, RangeRegistry      # noqa: E402
from dbc_compositor.merge import merge_dbc_outputs                    # noqa: E402
from dbc_compositor.framexml import resolve_overrides                 # noqa: E402
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
            registry.add(m.mod, dbc_name, ranges)          # check A
        recipe = _import_recipe(m)
        produced = recipe.build(m.sources)
        # check B precondition: every produced DBC is declared in id_ranges
        for dbc_name in produced:
            if dbc_name not in m.id_ranges:
                raise SystemExit(
                    f"{m.mod} recipe produced {dbc_name} but MANIFEST.toml "
                    f"declares no [id_ranges] for it"
                )
        outputs[m.mod] = produced
    return merge_dbc_outputs(outputs)


def collect_framexml() -> dict[str, str]:
    entries = []
    for manifest_path in sorted(MODULES.glob("*/client/MANIFEST.toml")):
        import tomllib
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
        # DBCs (provenance: these are recipe blobs, never read from a .dbc file)
        for dbc_name, blob in dbcs.items():
            files[f"DBFilesClient\\{dbc_name}"] = blob
        # AddOn files
        for f in addon.rglob("*"):
            if f.is_file():
                rel = f.relative_to(tmp).as_posix().replace("/", "\\")
                files[f"Interface\\AddOns\\{rel}"] = f.read_bytes()
        # FrameXML overrides
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
```

- [ ] **Step 2: Smoke-run the composer + packer end-to-end**

Run:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
python tot/client-patch/compose-tot-addon.py
python tot/client-patch/pack-mpq.py --version 1.0.0 --out tot/client-patch/build/patch-ZZ-tot-1.0.0.MPQ
```
Expected: `DONE: .../patch-ZZ-tot-1.0.0.MPQ (<nonzero> bytes, sha256=...)`. Requires StormLib. If `libstorm` is missing, the run aborts with the install hint from `mpq_pack.py` — install via `brew install stormlib` and retry.

- [ ] **Step 3: Verify the MPQ contains the expected members**

Run (Python one-liner using StormLib to list, or extract + check):
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
python - <<'PY'
# Minimal: assert the DBCs are present by re-reading via StormLib list.
# If listing is awkward, assert size > AddOn-only baseline as a proxy.
from pathlib import Path
p = Path("tot/client-patch/build/patch-ZZ-tot-1.0.0.MPQ")
assert p.stat().st_size > 0
print("MPQ built:", p.stat().st_size, "bytes")
PY
```
Expected: prints a non-zero size.

- [ ] **Step 4: Commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git add tot/client-patch/pack-mpq.py
git commit -m "feat(client-patch): pack-mpq.py Stage B orchestrator

Discover MANIFEST.toml -> RangeRegistry check A -> run recipes -> merge ->
provenance -> @TOT_VERSION@ substitution -> pack DBC+AddOn+FrameXML into one
MPQ via StormLib. --check builds twice and asserts identical sha256.

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Collision integration + determinism + golden regression tests

**Goal:** Top-level compositor tests: a synthetic second DBC contributor that *would* collide proves check A fires end-to-end; determinism via `--check`; and the golden guard that the composed `Spell.dbc`/`ItemSet.dbc` blobs still match the captured live bytes.

**Files:**
- Create: `tot/client-patch/tests/fixtures/fakemod/client/MANIFEST.toml`
- Create: `tot/client-patch/tests/fixtures/fakemod/client/build_dbc.py`
- Create: `tot/client-patch/tests/test_collision_integration.py`
- Create: `tot/client-patch/tests/test_pack_determinism.py`
- Create: `tot/client-patch/tests/test_golden_bracket_sets.py`

- [ ] **Step 1: Create the synthetic colliding fixture module**

Create `tot/client-patch/tests/fixtures/fakemod/client/MANIFEST.toml`:
```toml
[manifest]
mod = "fakemod"
[sources]
[recipe]
entry = "build_dbc"
[id_ranges]
# Deliberately OVERLAPS mod-bracket-sets Spell.dbc 64752..64827.
"Spell.dbc" = [{ min = 64800, max = 64900 }]
```

Create `tot/client-patch/tests/fixtures/fakemod/client/build_dbc.py`:
```python
# SPDX-License-Identifier: GPL-2.0-or-later
def build(sources):
    return {"Spell.dbc": b"FAKE"}
```

- [ ] **Step 2: Write the collision integration test**

Create `tot/client-patch/tests/test_collision_integration.py`:
```python
# SPDX-License-Identifier: GPL-2.0-or-later
"""Prove RangeRegistry fires when a real module would overlap.

We exercise the registry directly with the bracket-sets manifest + the fake
overlapping manifest (rather than dropping the fixture into modules/, which
would pollute a real build)."""
from pathlib import Path

import pytest

from dbc_compositor.manifest import load_manifest, RangeRegistry, CollisionError

REPO = Path(__file__).resolve().parents[3]
BRACKET = REPO / "modules" / "mod-bracket-sets" / "client" / "MANIFEST.toml"
FAKE = Path(__file__).resolve().parent / "fixtures" / "fakemod" / "client" / "MANIFEST.toml"


def test_real_bracket_sets_manifest_loads():
    m = load_manifest(BRACKET)
    assert m.mod == "mod-bracket-sets"
    assert m.id_ranges["Spell.dbc"] == [(64752, 64827)]


def test_overlap_with_bracket_sets_raises():
    reg = RangeRegistry()
    b = load_manifest(BRACKET)
    f = load_manifest(FAKE)
    for name, ranges in b.id_ranges.items():
        reg.add(b.mod, name, ranges)
    with pytest.raises(CollisionError, match="overlap"):
        for name, ranges in f.id_ranges.items():
            reg.add(f.mod, name, ranges)
```

- [ ] **Step 3: Write the determinism test**

Create `tot/client-patch/tests/test_pack_determinism.py`:
```python
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
```

- [ ] **Step 4: Write the golden regression test (compositor level)**

Create `tot/client-patch/tests/test_golden_bracket_sets.py`:
```python
# SPDX-License-Identifier: GPL-2.0-or-later
"""The non-negotiable guard: the compositor's bracket-sets DBC output stays
byte-identical to the live-shipped blobs (kb_99501ac5)."""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO / "tot" / "client-patch"))

import importlib.util  # noqa: E402
from dbc_compositor.manifest import load_manifest  # noqa: E402

BRACKET = REPO / "modules" / "mod-bracket-sets" / "client"


def _recipe():
    spec = importlib.util.spec_from_file_location("bs_recipe", BRACKET / "build_dbc.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_compositor_dbc_matches_golden():
    m = load_manifest(BRACKET / "MANIFEST.toml")
    out = _recipe().build(m.sources)
    assert out["ItemSet.dbc"] == (BRACKET / "golden" / "itemset.dbc.golden").read_bytes()
    assert out["Spell.dbc"] == (BRACKET / "golden" / "spell.dbc.golden").read_bytes()
```

- [ ] **Step 5: Run all compositor tests**

Run:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
python -m pytest tot/client-patch/tests/ -v
```
Expected: collision + golden tests PASS; determinism test PASS or SKIP (if StormLib absent).

- [ ] **Step 6: Run the FULL suite (library + recipe + compositor), confirm nothing regressed**

Run:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
python -m pytest tot/client-patch/lib/dbc_compositor/tests/ modules/mod-bracket-sets/client/tests/ tot/client-patch/tests/ -v
```
Expected: all green (StormLib-dependent cases may skip).

- [ ] **Step 7: Commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git add tot/client-patch/tests
git commit -m "test(client-patch): collision integration + determinism + golden guard

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: Retire the dead dbc-patch-builder + warforged scaffolding

**Goal:** Remove the now-superseded mod-bracket-sets-specific builder and the dead 0-byte warforged MPQ + stopgap packer. **Deletion of untracked/ambiguous files requires asking the user first per CLAUDE.md; these are git-tracked, so removal is a normal commit — but confirm with the user before running this task since it deletes the legacy builder.**

**Files:**
- Delete: `tot/client-patch/dbc-patch-builder/` (remaining: `build.py`, `check.py`, `pyproject.toml`, `README.md`, `scripts/`, `build/`, leftover `__init__.py`)
- Delete: `modules/mod-warforged/client/patch-W.MPQ` (0-byte)
- Delete: `modules/mod-warforged/tools/pack-mpq.py` (dead stub)

- [ ] **Step 1: Confirm the builder's logic is fully ported**

Run:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
test -f modules/mod-bracket-sets/client/build_dbc.py && echo "recipe exists"
test -f modules/mod-bracket-sets/client/golden/spell.dbc.golden && echo "golden exists"
python -m pytest modules/mod-bracket-sets/client/tests/ -q
```
Expected: "recipe exists", "golden exists", tests pass. Only proceed if all three hold.

- [ ] **Step 2: Salvage the old check.py drift logic if still referenced**

The old `check.py` was the determinism `--check` for the bracket-sets MPQ; `pack-mpq.py --check` supersedes it. Confirm nothing imports it:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
grep -rn "dbc_patch_builder" --include=*.py . | grep -v dbc-patch-builder/ || echo "no external refs"
```
Expected: "no external refs" (all consumers now use `dbc_compositor`).

- [ ] **Step 3: Remove the dead trees**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git rm -r tot/client-patch/dbc-patch-builder
git rm modules/mod-warforged/client/patch-W.MPQ
git rm modules/mod-warforged/tools/pack-mpq.py
```
(If `modules/mod-warforged/tools/` is now empty except `__pycache__`, also `git rm -r --cached` any tracked pycache and let .gitignore handle the rest.)

- [ ] **Step 4: Verify the full suite still passes**

Run:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
python -m pytest tot/client-patch/lib/dbc_compositor/tests/ modules/mod-bracket-sets/client/tests/ tot/client-patch/tests/ -v
```
Expected: all green.

- [ ] **Step 5: Commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git commit -m "chore(client-patch): retire dbc-patch-builder + dead warforged MPQ scaffold

dbc-patch-builder logic fully ported to dbc_compositor lib + mod-bracket-sets
recipe. mod-warforged ships zero DBC rows (its client assets are AddOn Lua via
Stage A); the 0-byte patch-W.MPQ and stopgap tools/pack-mpq.py were dead.

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: DBC-RANGES.md, player-install doc, CI hook

**Goal:** The project-wide DBC allocation map, the player-facing install doc, and the release CI hook.

**Files:**
- Create: `tot/client-patch/DBC-RANGES.md`
- Create: `docs/player-install.md`
- Create: `tot/release/build-mpq.sh`

- [ ] **Step 1: Write DBC-RANGES.md**

Create `tot/client-patch/DBC-RANGES.md`:
```markdown
# Threads of Time — DBC ID-range allocation

Each module shipping client DBC rows declares its owned ID ranges in
`modules/<mod>/client/MANIFEST.toml` under `[id_ranges]`. The Stage B
compositor (`pack-mpq.py`) fails loud if two modules claim overlapping ranges
for the same DBC file. This file is the human-readable map; the manifests are
the source of truth. Update this table in the same commit as any range change.

| DBC file       | Range            | Owner            | Notes |
|----------------|------------------|------------------|-------|
| ItemSet.dbc    | 90100 – 90199    | mod-bracket-sets | 90101 fallback + 90111..90193 (27 class+spec) |
| Spell.dbc      | 64752 – 64827    | mod-bracket-sets | 54 marker-spell name/description overrides (sparse) |

## Reserved / aspirational (not yet shipped)

- `SpellItemEnchantment.dbc` 4080000+ was reserved for mod-warforged bonus-socket
  enchants, but mod-warforged ships zero DBC rows as of 1.0.0 (its client assets
  are AddOn Lua). No manifest claims this range. Re-add a manifest entry here if
  mod-warforged ever ships DBC rows.

## Adding a new DBC contributor

1. Pick an unused range for each DBC file you touch.
2. Add `[id_ranges]` to your `modules/<mod>/client/MANIFEST.toml`.
3. Add a row to the table above.
4. `python tot/client-patch/pack-mpq.py --version 0.0.0-dev` — a clean run proves
   no overlap; an overlap aborts with a message naming both modules.
```

- [ ] **Step 2: Write the player-install doc**

Create `docs/player-install.md`:
```markdown
# Threads of Time — Player Install

Threads of Time does not distribute the WoW client. You bring your own legitimate
**WoW 3.3.5a (build 12340)** client. ToT ships one additive client patch.

## Requirements

| Item | Source |
|---|---|
| WoW 3.3.5a client (build 12340) | Your own copy |
| ToT client patch | GitHub Releases: `patch-ZZ-tot-<version>.MPQ` |
| Realmlist | From your server operator |

## Install (three steps)

1. **Download** `patch-ZZ-tot-<version>.MPQ` from the release page, then
   **rename it to `patch-ZZ.MPQ`** and copy it into your client's `Data/` folder:
   - Windows: `<WoW>\Data\patch-ZZ.MPQ`
   - macOS / Wine: same relative path
   The patch is additive — it does not modify stock client files and is
   reversible by deleting it.
2. **Edit** `Data/<locale>/realmlist.wtf` (e.g. `Data/enUS/realmlist.wtf`):
   ```
   set realmlist your.server.example.com
   ```
3. **Launch `Wow.exe`** and log in with the account your operator gave you.

### One-time cache clear (installing over an existing client)

If you played this client *before* installing the patch, clear the item cache once
so tier-set tooltips render correctly:
```
Delete (or move aside): Cache/WDB/enUS/itemcache.wdb
```
The client rebuilds it on next login. (Skip if this is a fresh client.)

## Verify the install

- **Login screen:** reads "Threads of Time <version>" with the fan-project
  disclaimer. If you see stock text, the MPQ is not loaded (re-check the
  filename and `Data/` location).
- **In-game, hover a Bracket 1 tier piece:** the tooltip shows spec-specific
  set-bonus rows. Stock "Vestiges of Blackfathom (0/2)" with no bonus rows means
  the MPQ is not loaded.
- **Loot a Bracket 1 item:** roughly 10% chance of an orange "Warforged" tag.

## Upgrading

1. Delete the old `patch-ZZ.MPQ`.
2. Download the new `patch-ZZ-tot-<version>.MPQ`, rename to `patch-ZZ.MPQ`,
   drop into `Data/`.

(No realmlist change needed on upgrade.)

## Common errors

| Symptom | Cause | Fix |
|---|---|---|
| `Login server unavailable` | Wrong realmlist or server down | Re-check `realmlist.wtf`; contact operator |
| Tooltips show no bonus rows / no Warforged tag | MPQ not loaded | Re-check filename (`patch-ZZ.MPQ`) + location (`Data/`) |
| Tier tooltips empty after a patch update | Stale item cache | Clear `Cache/WDB/enUS/itemcache.wdb` |
| `Disconnected: client/server version mismatch` | Patch version too old | Download the current `patch-ZZ-tot-<version>.MPQ` |

## Legal

Threads of Time is an unofficial, non-commercial fan project. It is not
affiliated with, endorsed by, or sponsored by Blizzard Entertainment, Inc.
World of Warcraft® and Wrath of the Lich King® are trademarks of Blizzard
Entertainment. All Blizzard intellectual property remains the property of
Blizzard Entertainment.
```

- [ ] **Step 3: Write the CI hook**

Create `tot/release/build-mpq.sh`:
```bash
#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Build the unified ToT client MPQ (Stage A + Stage B) + determinism gate.
# Usage: tot/release/build-mpq.sh <version>   (default 0.0.0-dev)
set -euo pipefail

VERSION="${1:-0.0.0-dev}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CP="$REPO/tot/client-patch"
OUT="$CP/build/patch-ZZ-tot-$VERSION.MPQ"

echo "== Stage A: compose AddOn =="
python3 "$CP/compose-tot-addon.py"

echo "== Stage B: pack MPQ ($VERSION) =="
python3 "$CP/pack-mpq.py" --version "$VERSION" --out "$OUT"

echo "== Determinism gate =="
python3 "$CP/pack-mpq.py" --version "$VERSION" --check

echo "== sha256 =="
if command -v sha256sum >/dev/null; then sha256sum "$OUT"; else shasum -a 256 "$OUT"; fi
echo "OK: $OUT"
```

Make it executable:
```bash
chmod +x /Users/tbrack/Documents/Projects/threads-of-time/tot/release/build-mpq.sh
```

- [ ] **Step 4: Run the CI hook end-to-end (if StormLib present)**

Run:
```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
./tot/release/build-mpq.sh 1.0.0
```
Expected: Stage A composes, Stage B packs, determinism prints `DETERMINISTIC OK`, sha256 printed, `OK:` line. (Skip if StormLib absent; note it.)

- [ ] **Step 5: Commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git add tot/client-patch/DBC-RANGES.md docs/player-install.md tot/release/build-mpq.sh
git commit -m "docs(client-patch): DBC-RANGES map + player-install doc + build-mpq CI hook

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: State doc + kb update

**Goal:** Write `MPQ_COMPOSITOR_STATE.md` following the existing state-doc pattern, and update the START HERE kb with Thread M.

**Files:**
- Create: `MPQ_COMPOSITOR_STATE.md`
- Update: `kb_87a7eade` (via ninum-knowledge MCP)

- [ ] **Step 1: Read a sibling state doc for the pattern**

Run:
```bash
sed -n '1,60p' /Users/tbrack/Documents/Projects/threads-of-time/SUBSET_GATING_STATE.md
```
Match its structure (end-state summary, what shipped, file inventory, test counts, carry-forward, verification path).

- [ ] **Step 2: Write MPQ_COMPOSITOR_STATE.md**

Create `MPQ_COMPOSITOR_STATE.md` capturing: the shared `dbc_compositor` library + per-module recipe architecture; the manifest + RangeRegistry collision model (checks A + B + one-producer-per-file); the golden-file regression guard (with the recorded sha prefixes from Task 3); the branding overlay; FrameXML support; `pack-mpq.py` + `build-mpq.sh`; the `patch-ZZ` filename convention; that mod-warforged ships zero DBC rows; total test count across the three suites; and carry-forward items (the two client-verification pre-flights from design §5.2: glue FontString name + glue-AddOn loadability — to be resolved during the in-game test). Follow the sibling doc's headings.

- [ ] **Step 3: Commit the state doc**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git add MPQ_COMPOSITOR_STATE.md
git commit -m "docs: record mpq-compositor state (Plan 4 done)

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 4: Tag the completion point**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git tag mpq-compositor-complete
git log --oneline -1
```

- [ ] **Step 5: Update the START HERE kb**

Via ninum-knowledge MCP `update_knowledge_entry` on `kb_87a7eade`: add a "Thread M — ToT 1.0.0 MPQ compositor — Plan 4 COMPLETE" line under Active threads (tag `mpq-compositor-complete`, single DBC contributor for 1.0.0, collision machinery general, golden guard green, branding overlay shipped, two client-verification carry-forwards). Move the "Plan 4 (MPQ compositor)" mention out of "next" in Thread L. Keep the entry under ~1500 words.

- [ ] **Step 6: (Optional) dispatch knowledge-curator**

If the kb edit is non-trivial, dispatch the `knowledge-curator` agent to make the Thread M edit + verify no broken `[[ ]]` links, per project CLAUDE.md.

---

## Self-Review (completed by plan author)

**Spec coverage:**
- §2.1 two-stage pipeline → Tasks 1–8 (Stage A reused; Stage B built).
- §2.2 library/recipe split → Tasks 1 (lib), 3 (recipe).
- §3 manifest + recipe contract → Tasks 2, 3.
- §4.1 check A → Task 2; §4.2 check B precond → Task 7 (declared-range presence) + Task 3 (in-range test); §4.3 merge/dedup → Task 4; §4.4 DBC-RANGES.md → Task 10; §4.5 provenance audit → Task 7 (recipe-only blobs).
- §5.1 pack-mpq → Task 7; §5.2 branding → Task 6; §5.3 FrameXML → Task 5; §5.4 determinism → Tasks 7, 8; §5.5 version lockstep → Tasks 6 (token), 7 (substitution); §5.6 filename → Tasks 7, 10.
- §6 pre-flight: Spell IDs resolved (Task 3 manifest uses real 64752–64827); warforged reality resolved (Task 9 deletes dead scaffold, no recipe).
- §7 testing → Tasks 3, 8; golden guard → Tasks 3 (recipe-level) + 8 (compositor-level); in-game path → player-install doc (Task 10) + state-doc carry-forward (Task 11).
- §8 file layout → matches Tasks 1–11.
- §9 CI hook → Task 10. §10 deliverables → Tasks 1–11. §11 agent dispatch → noted in Task 9 (warforged owner) + Task 11 (curator).

**Placeholder scan:** No `TBD`/`TODO` left as deferred work. Two intentional `<RECORD>` tokens in Task 3's commit message are instructions to paste the captured sha prefixes — the values are produced by Step 1 of that task, not guessable in advance. The branding Lua's `@TOT_VERSION@` is a real substitution token, not a placeholder.

**Type consistency:** `load_manifest`/`Manifest`/`RangeRegistry`/`CollisionError` (manifest.py) used consistently in Tasks 2, 3, 7, 8. `merge_dbc_outputs`/`DuplicateProducerError` (merge.py) consistent Tasks 4, 7. `resolve_overrides`/`FrameXmlConflictError` (framexml.py) consistent Tasks 5, 7. Recipe contract `build(sources) -> {dbc_name: bytes}` consistent Tasks 3, 7, 8. `Manifest.id_ranges` is `dict[str, list[tuple[int,int]]]` everywhere it's read.

**One known boundary (documented, not a gap):** recipes return whole DBC blobs, not row lists, so cross-module row-level union of the *same* DBC file is out of scope for 1.0.0 (Task 4 enforces one-producer-per-file instead). This is honest given the existing encoders and the single-DBC-contributor reality; the RangeRegistry still provides the ID-overlap guarantee the spec's headline feature requires.
```
