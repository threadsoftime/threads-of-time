# Threads of Time — MPQ Compositor Complete

This file is a one-time record of the state at the `mpq-compositor-complete` tag.

## What works

- Shared `tot/client-patch/lib/dbc_compositor/` library with nine generic WDBC/MPQ building-block modules: `dbc_io`, `itemset_dbc`, `spell_dbc`, `bonus_map_parser`, `mpq_pack`, `manifest`, `merge`, `framexml`, `dbc_audit`
- Per-module recipes at `modules/<mod>/client/build_dbc.py` driven by `modules/<mod>/client/MANIFEST.toml`
- Two-phase collision model: check A (`RangeRegistry` rejects declared-range overlap for the same DBC file) + check B (`dbc_audit.assert_ids_in_ranges` rejects any produced blob row outside the module's declared ranges, enforcing the §10.3 stock-Blizzard-row gate); `merge.merge_dbc_outputs` enforces one-producer-per-DBC-file
- **mod-bracket-sets** client recipe: ItemSet.dbc (ranges 90100–90199; 28 rows = fallback 90101 + 27 class+spec 90111..90193) + Spell.dbc (three clusters 64854–64939, 67121–67268, 70724–70841; 54 marker overrides)
- **mod-warforged** client recipe: SpellItemEnchantment.dbc (ranges 70001–70063; 21 partial ToT rows per §10.3; server builds the full file separately via `tools/build-warforged-dbc.py`, untouched)
- Golden regression guard: `modules/mod-bracket-sets/client/golden/itemset.dbc.golden` (5981 bytes, sha256 prefix `2a5685d6`) + `modules/mod-bracket-sets/client/golden/spell.dbc.golden` (59693 bytes, sha256 prefix `476cedea`); recipe output is byte-locked
- Branding overlay: `tot/client-patch/branding/` GlueXML AddOn (login-screen version watermark + mandatory fan-project §10.4 disclaimer), composed via Stage A, `@TOT_VERSION@` substituted at pack time
- Priority-based FrameXML override resolver (`framexml.py`): present and tested; currently unused (all modules ship AddOn Lua)
- `tot/client-patch/pack-mpq.py` Stage B orchestrator: discover → check A → recipes → check B → merge → `@TOT_VERSION@` substitution → pack DBC + AddOn + FrameXML via StormLib → sha256; `--check` determinism gate
- `tot/release/build-mpq.sh` CI hook wired to `pack-mpq.py`
- Output: one `patch-ZZ-tot-<version>.MPQ` (~14.4 KB with 3 DBCs); deterministic (sha256 prefix `fe2c5c68` for v1.0.0); players rename to `patch-ZZ.MPQ`
- DBC-RANGES.md map + player-install doc published under `tot/client-patch/`
- Superseded `dbc-patch-builder` retired (Task 10)
- 70 tests passing: dbc_compositor lib 44 + mod-bracket-sets recipe 5 + mod-warforged recipe 5 + top-level client-patch integration (collision / stock-row / determinism / golden / branding) 9 + mod-warforged server-side 7 (untouched)

## What's deferred (carry-forward for the in-game test)

Two client-side pre-flights from design §5.2 remain open until a live ChromieCraft 3.3.5a client is available:

1. **Glue version FontString name** — `VersionLabel` is assumed as the global name for the login-screen branding overlay. Must be verified against the actual 3.3.5a GlueXML before shipping to players; if the name differs, a one-line fix in the branding AddOn resolves it.
2. **GlueXML AddOn loading on 3.3.5a** — whether glue-screen AddOns load on the ChromieCraft 3.3.5a build is unconfirmed. If they do not, branding must be shipped via the FrameXML-override rail (the resolver exists and is tested; wiring it up is a one-task change).
3. **Warforged client DBC necessity** — open question: does the 3.3.5a client require a client-side SpellItemEnchantment.dbc given that warforged enchant display is AddOn-driven via `WF_STAT_BUMPS`? If not, mod-warforged ships zero client DBC. The partial-row recipe (§10.3-safe default) is what ships until this is resolved via live client inspection.

## What's deferred to 1.1.0+

- Live Heimdal client-MPQ smoke test (patch-ZZ.MPQ drop into a test client, login, verify set bonuses display)
- Warforged enchant display verification on a real 3.3.5a client with the MPQ loaded
- Multi-module collision stress-test with Brackets 2–7 DBC data (data not authored yet; bracket_set_bonus_map rows for brackets 2–7 are empty as of 2026-05-19)

## File inventory

| Path | Purpose |
|---|---|
| `tot/client-patch/lib/dbc_compositor/src/dbc_compositor/` | Shared compositor library (9 modules) |
| `tot/client-patch/lib/dbc_compositor/tests/` | Library unit tests (44) |
| `tot/client-patch/pack-mpq.py` | Stage B orchestrator |
| `tot/client-patch/branding/` | GlueXML branding AddOn + README |
| `tot/client-patch/DBC-RANGES.md` | Module → DBC-ID range map |
| `tot/release/build-mpq.sh` | CI hook |
| `modules/mod-bracket-sets/client/build_dbc.py` | Bracket-sets recipe |
| `modules/mod-bracket-sets/client/MANIFEST.toml` | Bracket-sets range declarations |
| `modules/mod-bracket-sets/client/golden/` | Byte-locked golden blobs (2 files) |
| `modules/mod-bracket-sets/client/tests/test_build_dbc.py` | Recipe tests (5) |
| `modules/mod-warforged/client/build_dbc.py` | Warforged recipe |
| `modules/mod-warforged/client/MANIFEST.toml` | Warforged range declarations |
| `modules/mod-warforged/client/tests/test_build_dbc.py` | Recipe tests (5) |
| `tot/client-patch/tests/` | Top-level integration tests (9) |

## Test counts

| Suite | Count | Status |
|---|---|---|
| `dbc_compositor` lib | 44 | green |
| mod-bracket-sets recipe | 5 | green |
| mod-warforged recipe | 5 | green |
| client-patch integration | 9 | green |
| mod-warforged server-side (untouched) | 7 | green |
| **Total** | **70** | **all green** |

## Known issues

1. **No live client smoke test.** The three carry-forward items above are spec-level open questions (§5.2), not code defects. All compositor logic is unit- and integration-tested; runtime verification on a 3.3.5a client is deferred.
2. **Brackets 2–7 DBC data absent.** The collision machinery and range declarations exist for future brackets, but `bracket_set_bonus_map` / `itemset_dbc` / `item_template` rows for brackets 2–7 are not authored as of 2026-05-19. Adding them in 1.1.0 requires only: authoring the DB rows, running the recipe, capturing new goldens.
3. **7 pre-existing harness test failures** in `test_mcp_auth.py` / `test_mcp_parity.py` / `test_mcp_server.py` are unrelated to Plan 4 (missing `asyncio_mode = "auto"` in `tot/harness/pyproject.toml`). Pre-date Plan 3; tracked separately.

## What's pinned

- Tag: `mpq-compositor-complete`
- MPQ sha256 prefix `fe2c5c68` for v1.0.0 (determinism gate certified at `e738547ff`)
- ItemSet.dbc golden sha256 prefix `2a5685d6` (5981 bytes)
- Spell.dbc golden sha256 prefix `476cedea` (59693 bytes)
- All 28 bracket-sets ItemSet rows + 54 Spell marker-override rows in ranges declared by `modules/mod-bracket-sets/client/MANIFEST.toml`
- 21 warforged SpellItemEnchantment partial rows in range declared by `modules/mod-warforged/client/MANIFEST.toml`

## Verification path

```bash
# Run all compositor tests
cd tot/client-patch/lib/dbc_compositor && python -m pytest tests/ -q
cd modules/mod-bracket-sets/client && python -m pytest tests/ -q
cd modules/mod-warforged/client && python -m pytest tests/ -q
cd tot/client-patch && python -m pytest tests/ -q

# Build the MPQ and verify determinism
cd tot/client-patch && python pack-mpq.py --version 1.0.0 --out build/
python pack-mpq.py --version 1.0.0 --out build/ --check   # determinism gate

# Inspect the output
sha256sum build/patch-ZZ-tot-1.0.0.MPQ   # prefix should be fe2c5c68
```

## Next plans

Plan 5 (operator install stack) picks up the deferred Heimdal e2e integration (Plan 2 Task 38 + Plan 3 §9.3) and can now proceed. The client-side pre-flights above (§5.2) are inputs to Plan 5's first-boot bootstrap sequence.

Plan 6 (release pipeline) adds `release.sh`, GHCR push, GitHub Release creation, and nightly AC drift CI; it consumes the `build-mpq.sh` hook added in Plan 4.
