# tests/firstboot/test_apply_order.py
import tomllib
from pathlib import Path
import pytest
from tot.deploy.firstboot.apply_content_sql import load_apply_order, DB_TO_DATABASE

REPO = Path(__file__).resolve().parents[2]
SQL_ROOT = REPO / "tot" / "content" / "sql"

def test_manifest_lists_every_on_disk_sql_file():
    listed = {e["file"] for e in load_apply_order(SQL_ROOT / "apply-order.toml")}
    on_disk = {str(p.relative_to(SQL_ROOT)) for p in SQL_ROOT.rglob("*.sql")}
    assert on_disk == listed, f"manifest/disk mismatch: {on_disk ^ listed}"

def test_every_db_maps_to_a_real_database():
    for e in load_apply_order(SQL_ROOT / "apply-order.toml"):
        assert e["db"] in DB_TO_DATABASE

def test_order_is_preserved():
    entries = load_apply_order(SQL_ROOT / "apply-order.toml")
    assert entries[0]["file"].startswith("bracket1/")
    assert entries[-1]["file"] == "bracket1/characters_wipe_bots.sql"
