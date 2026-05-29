"""Apply ToT content SQL in the order declared by apply-order.toml."""
from __future__ import annotations
import argparse, subprocess, sys, tomllib
from pathlib import Path

DB_TO_DATABASE = {"world": "tot_world", "characters": "tot_characters", "auth": "tot_auth"}

def load_apply_order(manifest: Path) -> list[dict]:
    with open(manifest, "rb") as fh:
        data = tomllib.load(fh)
    return data.get("apply", [])

def apply(sql_root: Path, host: str, port: str, user: str, password: str) -> None:
    for entry in load_apply_order(sql_root / "apply-order.toml"):
        sql_file = sql_root / entry["file"]
        database = DB_TO_DATABASE[entry["db"]]
        print(f"  applying {entry['file']} -> {database}", flush=True)
        with open(sql_file, "rb") as fh:
            subprocess.run(
                ["mysql", f"-h{host}", f"-P{port}", f"-u{user}", f"-p{password}", database],
                stdin=fh, check=True,
            )

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--sql-root", type=Path, required=True)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", default="3306")
    ap.add_argument("--user", required=True)
    ap.add_argument("--password", required=True)
    args = ap.parse_args()
    apply(args.sql_root, args.host, args.port, args.user, args.password)
    return 0

if __name__ == "__main__":
    sys.exit(main())
