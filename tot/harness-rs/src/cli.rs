//! CLI definitions — `harness-rs serve | mint-token | validate`.
//!
//! Port of `harness_daemon/main.py` (click group + 3 commands).
//! Uses clap's derive API with the `env` feature enabled.

use clap::{Parser, Subcommand};

// ── Top-level parser ──────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(name = "harness-rs", about = "Harness-daemon Rust implementation")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

// ── Subcommands ───────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the harness-daemon HTTP server.
    Serve(ServeArgs),

    /// Print a YAML token entry for appending to tokens.yaml.
    #[command(name = "mint-token")]
    MintToken(MintTokenArgs),

    /// Validate a tokens.yaml config and exit.
    Validate(ValidateArgs),
}

// ── serve args ────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct ServeArgs {
    /// Path to tokens.yaml
    #[arg(long, default_value = "/etc/harness/tokens.yaml")]
    pub config: String,

    /// MySQL host
    #[arg(long, default_value = "127.0.0.1")]
    pub mysql_host: String,

    /// MySQL port
    #[arg(long, default_value_t = 3306)]
    pub mysql_port: u16,

    /// MySQL user
    #[arg(long, default_value = "root")]
    pub mysql_user: String,

    /// MySQL root password.  Also read from AC_MYSQL_PASSWORD env var.
    /// An empty value means "no DB client" (mirrors `main.py:53`).
    #[arg(long, env = "AC_MYSQL_PASSWORD", default_value = "")]
    pub mysql_password: String,
}

// ── mint-token args ───────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct MintTokenArgs {
    /// Token identity label (e.g. `bot.Krak`)
    #[arg(long)]
    pub identity: String,

    /// Scope patterns (repeat flag for multiple: `--scope gm.* --scope obs.*`)
    #[arg(long, num_args = 1..)]
    pub scope: Vec<String>,

    /// Optional GUID to bind this token to a specific bot
    #[arg(long)]
    pub bound_to_guid: Option<i64>,

    /// Mark this token as an augmented-bot token
    #[arg(long, default_value_t = false)]
    pub augmented: bool,

    /// Optional human note stored in the YAML entry
    #[arg(long)]
    pub note: Option<String>,
}

// ── validate args ─────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct ValidateArgs {
    /// Path to tokens.yaml
    #[arg(long, default_value = "/etc/harness/tokens.yaml")]
    pub config: String,
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::sync::Mutex;

    /// Global lock: env var mutation via set_var/remove_var must be serialized
    /// across the test binary because process env is shared across threads.
    /// Any test that reads or writes AC_MYSQL_PASSWORD must hold this lock.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// W2 env-wiring: parse `serve` with AC_MYSQL_PASSWORD set in env,
    /// no `--mysql-password` CLI arg → parsed password == "test".
    ///
    /// Uses std::env::set_var because clap `env` reads std::env at parse time.
    /// The ENV_MUTEX serializes this test against `serve_defaults_parse` so the
    /// injected env var cannot leak across parallel test threads.
    #[test]
    fn serve_reads_mysql_password_from_env() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // Ensure clean state going in
        unsafe { std::env::remove_var("AC_MYSQL_PASSWORD") };
        unsafe { std::env::set_var("AC_MYSQL_PASSWORD", "test") };

        let args = Cli::parse_from(["harness-rs", "serve",
            "--config", "/tmp/test.yaml"]);

        // Restore immediately so other tests are unaffected
        unsafe { std::env::remove_var("AC_MYSQL_PASSWORD") };

        match args.command {
            Command::Serve(s) => {
                assert_eq!(s.mysql_password, "test",
                    "mysql_password must come from AC_MYSQL_PASSWORD env");
            }
            other => panic!("expected Serve, got {other:?}"),
        }
    }

    /// Default values parse correctly when no flags given.
    /// Holds ENV_MUTEX to prevent interference from the env-wiring test.
    #[test]
    fn serve_defaults_parse() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // Ensure AC_MYSQL_PASSWORD is not set — clap would pick it up otherwise
        unsafe { std::env::remove_var("AC_MYSQL_PASSWORD") };

        let args = Cli::parse_from(["harness-rs", "serve"]);
        match args.command {
            Command::Serve(s) => {
                assert_eq!(s.config,       "/etc/harness/tokens.yaml");
                assert_eq!(s.mysql_host,   "127.0.0.1");
                assert_eq!(s.mysql_port,   3306);
                assert_eq!(s.mysql_user,   "root");
                assert_eq!(s.mysql_password, "",
                    "default mysql_password must be empty when AC_MYSQL_PASSWORD is unset");
            }
            other => panic!("expected Serve, got {other:?}"),
        }
    }

    /// mint-token: identity + scope parse correctly.
    #[test]
    fn mint_token_parses_identity_and_scope() {
        let args = Cli::parse_from([
            "harness-rs", "mint-token",
            "--identity", "bot.Krak",
            "--scope", "bot.*",
            "--scope", "obs.*",
        ]);
        match args.command {
            Command::MintToken(m) => {
                assert_eq!(m.identity, "bot.Krak");
                assert_eq!(m.scope, vec!["bot.*", "obs.*"]);
                assert!(m.bound_to_guid.is_none());
                assert!(!m.augmented);
                assert!(m.note.is_none());
            }
            other => panic!("expected MintToken, got {other:?}"),
        }
    }

    /// validate subcommand parses config path.
    #[test]
    fn validate_parses_config_path() {
        let args = Cli::parse_from(["harness-rs", "validate",
            "--config", "/tmp/tokens.yaml"]);
        match args.command {
            Command::Validate(v) => {
                assert_eq!(v.config, "/tmp/tokens.yaml");
            }
            other => panic!("expected Validate, got {other:?}"),
        }
    }
}
