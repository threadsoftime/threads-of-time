# tools/dbc-patch-builder/tests/test_bonus_map_parser.py
from pathlib import Path
from dbc_patch_builder.bonus_map_parser import parse_bonus_map_seed, BonusRow

REPO_ROOT = Path(__file__).resolve().parents[4]
SEED_SQL = REPO_ROOT / "modules" / "mod-bracket-sets" / "data" / "sql" / "world" / "2026_05_13_01_bracket_set_bonus_map_seed.sql"


def test_seed_file_exists():
    assert SEED_SQL.exists(), f"missing seed at {SEED_SQL}"


def test_parses_54_rows():
    rows = parse_bonus_map_seed(SEED_SQL)
    assert len(rows) == 54


def test_row_shape():
    rows = parse_bonus_map_seed(SEED_SQL)
    arms_2pc = next(r for r in rows if r.class_id == 1 and r.spec_id == 1 and r.threshold == 2)
    assert arms_2pc.itemset_id == 90101
    assert arms_2pc.spell_id == 64938
    assert arms_2pc.bracket_min == 25
    assert arms_2pc.bracket_max == 34
    assert arms_2pc.display_name == "Sundered Resolve"


def test_all_thresholds_are_2_or_4():
    rows = parse_bonus_map_seed(SEED_SQL)
    assert all(r.threshold in (2, 4) for r in rows)


def test_27_class_spec_pairs():
    rows = parse_bonus_map_seed(SEED_SQL)
    pairs = {(r.class_id, r.spec_id) for r in rows}
    assert len(pairs) == 27  # 9 classes x 3 specs


def test_each_class_spec_has_2pc_and_4pc():
    rows = parse_bonus_map_seed(SEED_SQL)
    by_cs = {}
    for r in rows:
        by_cs.setdefault((r.class_id, r.spec_id), set()).add(r.threshold)
    for (cls, spec), thresholds in by_cs.items():
        assert thresholds == {2, 4}, f"class={cls} spec={spec} has {thresholds}, expected {{2,4}}"
