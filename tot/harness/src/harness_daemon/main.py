"""Entry point: load config, build app, launch uvicorn, handle SIGHUP."""

from __future__ import annotations

import os
import signal
import sys
from pathlib import Path

import click
import uvicorn

from .ac_client import ACClient
from .app import build_app
from .config import load_config_from_path


_app_state: dict = {"reload_requested": False}


def _on_sighup(signum, frame) -> None:
    # uvicorn handles graceful shutdown on SIGTERM. SIGHUP triggers a
    # config reload by re-execing the process — simplest correct
    # behavior given the in-memory immutable config.
    _app_state["reload_requested"] = True
    print("[harness-daemon] SIGHUP received — re-exec to reload config")
    os.execv(sys.executable, [sys.executable, "-m", "harness_daemon.main", "serve"] + sys.argv[2:])


@click.group()
def cli() -> None:
    pass


@cli.command()
@click.option("--config", "config_path",
              default="/etc/harness/tokens.yaml",
              show_default=True, help="Path to tokens.yaml")
@click.option("--mysql-host",    default="127.0.0.1", show_default=True)
@click.option("--mysql-port",    default=3306, type=int, show_default=True)
@click.option("--mysql-user",    default="root", show_default=True)
@click.option("--mysql-password", default=os.environ.get("AC_MYSQL_PASSWORD", ""),
              help="MySQL root password (or set AC_MYSQL_PASSWORD env)")
def serve(config_path, mysql_host, mysql_port, mysql_user, mysql_password) -> None:
    """Run the harness-daemon HTTP server."""
    from .db_client import DBClient
    cfg = load_config_from_path(config_path)
    host, _, port = cfg.listen_address.partition(":")
    if not port:
        host, port = "0.0.0.0", host

    ac = ACClient(cfg.ac_bridge_url)
    db = DBClient(mysql_host, mysql_port, mysql_user, mysql_password) if mysql_password else None
    app = build_app(cfg, ac_client=ac, db_client=db)

    signal.signal(signal.SIGHUP, _on_sighup)
    print(f"[harness-daemon] starting on {host}:{port}, ac_bridge={cfg.ac_bridge_url}, "
          f"mysql={'wired' if db else 'unconfigured'}")
    uvicorn.run(app, host=host, port=int(port), access_log=False)


@cli.command(name="mint-token")
@click.option("--identity", required=True)
@click.option("--scope", multiple=True, required=True,
              help="Scope pattern (repeat for multiple)")
@click.option("--bound-to-guid", type=int, default=None)
@click.option("--augmented", is_flag=True, default=False)
@click.option("--note", default="")
def mint_token(identity: str, scope: tuple[str, ...],
               bound_to_guid: int | None, augmented: bool, note: str) -> None:
    """Print a YAML token entry for appending to tokens.yaml."""
    import secrets
    import yaml as _yaml
    entry: dict = {
        "token":    secrets.token_urlsafe(24),
        "identity": identity,
        "scope":    list(scope),
    }
    if bound_to_guid is not None:
        entry["bound_to_guid"] = bound_to_guid
    if augmented:
        entry["augmented"] = True
    if note:
        entry["note"] = note
    print(_yaml.safe_dump([entry], sort_keys=False), end="")


if __name__ == "__main__":
    cli()
