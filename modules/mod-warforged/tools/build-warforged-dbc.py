#!/usr/bin/env python3
"""Build patched SpellItemEnchantment.dbc with mod-warforged enchant rows.

Reads:
  - data/csv/warforged_enchants.csv (21 calibrated rows, IDs 70001..70063)
  - data/dbc/input/SpellItemEnchantment.dbc (pristine 3.3.5a, sha256 baked below)

Writes:
  - data/dbc/SpellItemEnchantment.dbc          (server-side, AC loads this)
  - build/mpq-staging/DBFilesClient/SpellItemEnchantment.dbc (client-side, Task 17 packs to MPQ)

WotLK 3.3.5a SpellItemEnchantment.dbc schema (38 uint32 fields, 152 bytes/record):
  0:    ID
  1:    charges
  2-4:  effect[3]            (5 = ITEM_ENCHANTMENT_TYPE_STAT)
  5-7:  effectPointsMin[3]   (amount; what we calibrated)
  8-10: effectPointsMax[3]   (set equal to Min for fixed-value stat enchants)
  11-13:effectArg[3]         (ITEM_MOD_* stat id — STR=4, STA=7, CRIT=32, etc.)
  14-29:name_lang[16]        (16 localised name string offsets — index 0 = enUS)
  30:   name_lang_flags
  31:   itemVisualID         (0 = no particle effect)
  32:   flags                (0)
  33:   src_itemID           (0 = proc-generated)
  34:   condition_id         (0 = unconditional)
  35:   required_skill_id    (0)
  36:   required_skill_rank  (0)
  37:   required_level       (0)

Source struct (verified against AC):
  src/server/shared/DataStores/DBCStructure.h
  -> struct SpellItemEnchantmentEntry  (38 fields total counting commented-out
     amount2[3] and descriptionFlags that AC skips but DBC stores).

Header (DBC 0x57 0x44 0x42 0x43 = "WDBC", little-endian):
  uint32 magic        ("WDBC")
  uint32 record_count
  uint32 field_count  (38)
  uint32 record_size  (152)
  uint32 string_block_size

Strings live in a single concatenated null-terminated block immediately after
the records. Each name_lang[i] field is an offset (in bytes) into that block.
Offset 0 always points to an initial empty string (just a "\\0").
"""
from __future__ import annotations

import csv
import hashlib
import struct
import sys
from pathlib import Path

# Expected sha256 of the pristine input. If this changes upstream we want a
# loud failure rather than silently building against a different file.
EXPECTED_INPUT_SHA256 = (
    "6c8e8c7d6bc2030500a0687a2975e7f81eb1dc2a1da6939f7ddc64e55ce964de"
)

# DBC format constants (WotLK 3.3.5a SpellItemEnchantment.dbc).
DBC_MAGIC = b"WDBC"
FIELD_COUNT = 38
RECORD_SIZE = 152  # 38 * 4
HEADER_SIZE = 20

# Field offsets within a 38-uint32 record (verified against AC struct).
F_ID = 0
F_CHARGES = 1
F_EFFECT_BASE = 2          # 2..4
F_EFFECT_MIN_BASE = 5      # 5..7
F_EFFECT_MAX_BASE = 8      # 8..10
F_EFFECT_ARG_BASE = 11     # 11..13
F_NAME_LANG_BASE = 14      # 14..29 (16 locales)
F_NAME_LANG_FLAGS = 30
F_ITEM_VISUAL_ID = 31
F_FLAGS = 32
F_SRC_ITEM_ID = 33
F_CONDITION_ID = 34
F_REQUIRED_SKILL_ID = 35
F_REQUIRED_SKILL_RANK = 36
F_REQUIRED_LEVEL = 37

# enUS locale slot.
LOCALE_ENUS = 0


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def parse_csv(csv_path: Path) -> list[dict]:
    """Parse warforged_enchants.csv, skipping comment lines."""
    rows: list[dict] = []
    with csv_path.open(newline="", encoding="utf-8") as f:
        # Strip comment lines (#...) before handing to csv.reader; the file has
        # a commented-out header on line 23.
        reader = csv.reader(
            line for line in f if line.strip() and not line.lstrip().startswith("#")
        )
        for parts in reader:
            if len(parts) != 14:
                raise ValueError(f"expected 14 columns, got {len(parts)}: {parts}")
            rows.append({
                "id": int(parts[0]),
                "band": parts[1],
                "quality": int(parts[2]),
                "band_label": parts[3],
                "description": parts[4],
                "e1_type": int(parts[5]),
                "e1_amount": int(parts[6]),
                "e1_stat": int(parts[7]),
                "e2_type": int(parts[8]),
                "e2_amount": int(parts[9]),
                "e2_stat": int(parts[10]),
                "e3_type": int(parts[11]),
                "e3_amount": int(parts[12]),
                "e3_stat": int(parts[13]),
            })
    return rows


def build_record(row: dict, name_offset: int) -> bytes:
    """Pack one 38-uint32 row according to the AC schema."""
    fields = [0] * FIELD_COUNT
    fields[F_ID] = row["id"]
    fields[F_CHARGES] = 0

    fields[F_EFFECT_BASE + 0] = row["e1_type"]
    fields[F_EFFECT_BASE + 1] = row["e2_type"]
    fields[F_EFFECT_BASE + 2] = row["e3_type"]

    fields[F_EFFECT_MIN_BASE + 0] = row["e1_amount"]
    fields[F_EFFECT_MIN_BASE + 1] = row["e2_amount"]
    fields[F_EFFECT_MIN_BASE + 2] = row["e3_amount"]

    # For fixed-value stat enchants, max == min.
    fields[F_EFFECT_MAX_BASE + 0] = row["e1_amount"]
    fields[F_EFFECT_MAX_BASE + 1] = row["e2_amount"]
    fields[F_EFFECT_MAX_BASE + 2] = row["e3_amount"]

    fields[F_EFFECT_ARG_BASE + 0] = row["e1_stat"]
    fields[F_EFFECT_ARG_BASE + 1] = row["e2_stat"]
    fields[F_EFFECT_ARG_BASE + 2] = row["e3_stat"]

    # All 16 locale slots point at the same string for now. This matches what
    # AC's loader expects (it reads slot 0 for the active locale; the rest are
    # ignored unless server.conf overrides DBC.Locale).
    for i in range(16):
        fields[F_NAME_LANG_BASE + i] = name_offset

    fields[F_NAME_LANG_FLAGS] = 0xFFFFFFFF  # standard value for all-locales-set
    fields[F_ITEM_VISUAL_ID] = 0
    fields[F_FLAGS] = 0
    fields[F_SRC_ITEM_ID] = 0
    fields[F_CONDITION_ID] = 0
    fields[F_REQUIRED_SKILL_ID] = 0
    fields[F_REQUIRED_SKILL_RANK] = 0
    fields[F_REQUIRED_LEVEL] = 0

    # struct cast — uint32 little-endian.
    return struct.pack(f"<{FIELD_COUNT}I", *fields)


def emit_lua_stat_bumps_table(rows: list[dict], output_path: Path) -> None:
    """Write a Lua table mapping each Warforged enchant ID to its stat bumps.

    Format:
        WF_STAT_BUMPS = {
            [70001] = { str=1, sta=1, crit=1 },
            ...
        }

    Consumed by the FrameXML override (data/lua/GameTooltip.lua / ItemRef.lua)
    so client-side tooltip code can fold the enchant bumps into the displayed
    base stat lines. v1.0 of the override does not yet use this table (the
    folding step is deferred to v1.1) but we emit it now so the build pipeline
    stays in sync with the CSV from day one — adding the consumer later
    doesn't require re-running the build.

    Only effect_type == 5 (ITEM_ENCHANTMENT_TYPE_STAT) rows are folded. The
    stat-id -> short-name mapping mirrors warforged_enchants.csv column docs:
        3=AGI, 4=STR, 5=INT, 6=SPI, 7=STA, 31=HIT_RATING,
        32=CRIT_RATING, 35=HASTE_RATING
    """
    stat_names = {
        3: "agi", 4: "str", 5: "int", 6: "spi", 7: "sta",
        31: "hit", 32: "crit", 35: "haste",
    }

    lines: list[str] = [
        "-- AUTO-GENERATED by tools/build-warforged-dbc.py from "
        "data/csv/warforged_enchants.csv",
        "-- DO NOT EDIT BY HAND. Re-run the build script to regenerate.",
        "--",
        "-- Maps each Warforged enchant ID (70001..70063) to the stat bumps it",
        "-- applies, so the FrameXML override can fold them inline into the",
        "-- displayed stat lines instead of showing them as a separate enchant.",
        "WF_STAT_BUMPS = {",
    ]
    for r in rows:
        triples = (
            (r["e1_type"], r["e1_amount"], r["e1_stat"]),
            (r["e2_type"], r["e2_amount"], r["e2_stat"]),
            (r["e3_type"], r["e3_amount"], r["e3_stat"]),
        )
        entries: list[str] = []
        for (etype, amount, stat_id) in triples:
            if etype == 5 and stat_id in stat_names:
                entries.append(f"{stat_names[stat_id]}={amount}")
        # f-string `{{` escapes to a single `{`; the literal " }," supplies
        # the closing brace and trailing comma.
        lines.append(f"    [{r['id']}] = {{ " + ", ".join(entries) + " },")
    lines.append("}")

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def patch_dbc(input_path: Path, csv_rows: list[dict]) -> bytes:
    """Read the pristine DBC and return patched bytes with N rows appended."""
    raw = input_path.read_bytes()
    if len(raw) < HEADER_SIZE:
        raise ValueError("input file shorter than DBC header")

    magic, rec_count, fld_count, rec_size, sb_size = struct.unpack(
        "<4sIIII", raw[:HEADER_SIZE]
    )
    if magic != DBC_MAGIC:
        raise ValueError(f"bad magic: {magic!r}")
    if fld_count != FIELD_COUNT:
        raise ValueError(f"unexpected field_count: {fld_count} (want {FIELD_COUNT})")
    if rec_size != RECORD_SIZE:
        raise ValueError(f"unexpected record_size: {rec_size} (want {RECORD_SIZE})")

    records_start = HEADER_SIZE
    records_end = records_start + rec_count * rec_size
    sb_start = records_end
    sb_end = sb_start + sb_size

    if sb_end != len(raw):
        raise ValueError(
            f"file size mismatch: header says {sb_end}, file is {len(raw)}"
        )

    original_records = raw[records_start:records_end]
    original_strings = raw[sb_start:sb_end]

    # Append a single shared "Warforged" string to the block. All 21 new rows
    # point at this offset. Doing this once (not 21 times) keeps the string
    # block compact and matches how Blizzard authors the source DBCs.
    description = "Warforged"
    new_str_bytes = description.encode("utf-8") + b"\x00"
    name_offset = len(original_strings)  # offset where the new string starts
    appended_strings = original_strings + new_str_bytes

    new_records = b"".join(build_record(row, name_offset) for row in csv_rows)

    new_rec_count = rec_count + len(csv_rows)
    new_sb_size = len(appended_strings)

    new_header = struct.pack(
        "<4sIIII",
        DBC_MAGIC,
        new_rec_count,
        FIELD_COUNT,
        RECORD_SIZE,
        new_sb_size,
    )

    return new_header + original_records + new_records + appended_strings


def main() -> int:
    # Resolve paths relative to the module root (parent of tools/).
    tools_dir = Path(__file__).resolve().parent
    module_root = tools_dir.parent

    csv_path = module_root / "data" / "csv" / "warforged_enchants.csv"
    input_dbc = module_root / "data" / "dbc" / "input" / "SpellItemEnchantment.dbc"
    out_server = module_root / "data" / "dbc" / "SpellItemEnchantment.dbc"
    out_client = (
        module_root / "build" / "mpq-staging" / "DBFilesClient" /
        "SpellItemEnchantment.dbc"
    )

    if not csv_path.exists():
        print(f"ERROR: missing CSV at {csv_path}", file=sys.stderr)
        return 2
    if not input_dbc.exists():
        print(f"ERROR: missing input DBC at {input_dbc}", file=sys.stderr)
        return 2

    actual_sha = sha256_of(input_dbc)
    if actual_sha != EXPECTED_INPUT_SHA256:
        print(
            f"ERROR: input DBC sha256 mismatch.\n"
            f"  expected: {EXPECTED_INPUT_SHA256}\n"
            f"  actual:   {actual_sha}\n"
            f"This means the upstream DBC changed; re-pull the source DBC and "
            f"update EXPECTED_INPUT_SHA256 deliberately.",
            file=sys.stderr,
        )
        return 3

    csv_rows = parse_csv(csv_path)
    print(f"loaded {len(csv_rows)} rows from CSV")
    if len(csv_rows) == 0:
        print("ERROR: CSV had no data rows", file=sys.stderr)
        return 4

    patched = patch_dbc(input_dbc, csv_rows)

    for out_path in (out_server, out_client):
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_bytes(patched)
        print(f"wrote {out_path} ({len(patched)} bytes)")

    # Emit the Lua stat-bumps table for the FrameXML override (v1.1 consumer).
    lua_stat_bumps = module_root / "data" / "lua" / "WarforgedStatBumps.lua"
    emit_lua_stat_bumps_table(csv_rows, lua_stat_bumps)
    print(f"wrote {lua_stat_bumps}")

    # Sanity readback.
    with out_server.open("rb") as f:
        hdr = f.read(HEADER_SIZE)
    magic, rc, fc, rs, sb = struct.unpack("<4sIIII", hdr)
    print(
        f"output header: magic={magic!r} records={rc} fields={fc} "
        f"record_size={rs} string_block_size={sb}"
    )

    input_size = input_dbc.stat().st_size
    delta = len(patched) - input_size
    str_bytes = len("Warforged") + 1  # +1 for null terminator
    expected_delta = len(csv_rows) * RECORD_SIZE + str_bytes
    print(
        f"size delta: +{delta} bytes "
        f"(expected +{expected_delta} = {len(csv_rows)}*{RECORD_SIZE} + "
        f"{str_bytes})"
    )
    if delta != expected_delta:
        print("ERROR: size delta mismatch", file=sys.stderr)
        return 5

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
