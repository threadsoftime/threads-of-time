//! Daemon-direct MySQL query client.
//!
//! Port of `harness_daemon/db_client.py`. Only pre-registered SELECT
//! templates are allowed — no free-form SQL. Each template declares its
//! parameter names and produces a parameterised statement via mysql_async
//! prepared-statement (`exec`) protocol.

use mysql_async::{
    prelude::Queryable,
    Opts, OptsBuilder, Pool, Row as MysqlRow, Value as MysqlValue,
};
use serde_json::{json, Map, Value};
use thiserror::Error;

// ── QueryTemplate ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct QueryTemplate {
    pub name:   &'static str,
    pub sql:    &'static str,
    pub db:     &'static str,
    pub params: &'static [&'static str],
}

// ── V1 template map ───────────────────────────────────────────────────────────

// VERBATIM port of db_client.py:32-120.  SQL strings must not be reformatted.
static V1_TEMPLATES: &[QueryTemplate] = &[
    QueryTemplate {
        name: "bracket_set_bonus_map_for",
        db:   "acore_world",
        sql: "SELECT itemset_id, threshold, class_id, spec_id, spell_id, \
               bracket_min, bracket_max, display_name \
              FROM bracket_set_bonus_map \
              WHERE itemset_id=%s AND class_id=%s AND spec_id=%s \
              ORDER BY threshold",
        params: &["itemset_id", "class_id", "spec_id"],
    },
    QueryTemplate {
        name: "item_template_set_pieces",
        db:   "acore_world",
        sql: "SELECT entry, name, ItemSet, class, subclass, quality, \
               InventoryType, RequiredLevel \
              FROM item_template \
              WHERE ItemSet=%s \
              ORDER BY entry",
        params: &["itemset_id"],
    },
    QueryTemplate {
        name: "bracketsets_diag",
        db:   "acore_world",
        sql: "SELECT spell_id, ScriptName \
              FROM spell_script_names \
              WHERE spell_id=%s",
        params: &["spell_id"],
    },
    QueryTemplate {
        name: "character_online",
        db:   "acore_characters",
        sql: "SELECT guid, name, race, class, level, online \
              FROM characters \
              WHERE guid=%s",
        params: &["guid"],
    },
    QueryTemplate {
        name: "character_online_by_class",
        db:   "acore_characters",
        sql: "SELECT guid, name, race, class, level, online \
              FROM characters \
              WHERE class=%s AND online=1 \
              ORDER BY level \
              LIMIT 20",
        params: &["class_id"],
    },
    QueryTemplate {
        name: "game_event_all",
        db:   "acore_world",
        // Schema verified 2026-05-30 via DESCRIBE acore_world.game_event.
        // Columns state/nextstart do NOT exist in this fork.
        // UNIX_TIMESTAMP converts timestamp columns to integer epoch-seconds.
        sql: "SELECT eventEntry, \
               UNIX_TIMESTAMP(start_time) AS start_time, \
               UNIX_TIMESTAMP(end_time) AS end_time, \
               occurence, length, holiday, holidayStage, \
               description, world_event, announce \
              FROM game_event \
              ORDER BY eventEntry",
        params: &[],
    },
];

// ── DbError ───────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum DbError {
    #[error("unknown template: {0}")]
    UnknownTemplate(String),
    #[error("{0}")]
    BadParams(String),
    #[error("mysql error: {0}")]
    Mysql(#[from] mysql_async::Error),
}

// ── validate_template (pure, unit-testable) ───────────────────────────────────

/// Validate `name` against the allowlist and check that all declared
/// parameter names are present in `params`.
///
/// On success returns `(template, ordered_param_values)`.
/// Pure function — no I/O, fully unit-testable.
pub fn validate_template(
    name: &str,
    params: &Value,
) -> Result<(QueryTemplate, Vec<Value>), DbError> {
    let tpl = V1_TEMPLATES
        .iter()
        .find(|t| t.name == name)
        .copied()
        .ok_or_else(|| DbError::UnknownTemplate(name.to_string()))?;

    let mut ordered = Vec::with_capacity(tpl.params.len());
    for &p in tpl.params {
        let val = params
            .get(p)
            .ok_or_else(|| DbError::BadParams(format!("missing param '{p}'")))?;
        ordered.push(val.clone());
    }

    Ok((tpl, ordered))
}

// ── mysql Value helpers ───────────────────────────────────────────────────────

/// Convert a `serde_json::Value` param into a `mysql_async::Value`.
///
/// Supported: null → NULL, bool → Int, i64/u64 → Int/UInt, f64 → Double,
/// string → Bytes. Arrays and objects are serialised to JSON strings
/// (covers edge cases in smoke-test params).
fn json_to_mysql(v: &Value) -> MysqlValue {
    match v {
        Value::Null               => MysqlValue::NULL,
        Value::Bool(b)            => MysqlValue::Int(if *b { 1 } else { 0 }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                MysqlValue::Int(i)
            } else if let Some(u) = n.as_u64() {
                MysqlValue::UInt(u)
            } else if let Some(f) = n.as_f64() {
                MysqlValue::Double(f)
            } else {
                MysqlValue::Bytes(n.to_string().into_bytes())
            }
        }
        Value::String(s)          => MysqlValue::Bytes(s.as_bytes().to_vec()),
        other                     => MysqlValue::Bytes(other.to_string().into_bytes()),
    }
}

/// Convert a `mysql_async::Value` cell into a `serde_json::Value`.
///
/// ints → Number, floats → Number, NULL → Null, Bytes → String (UTF-8) or
/// Number (when the text-protocol returns integers as decimal strings),
/// Date/Time → String representation.
fn mysql_to_json(v: MysqlValue) -> Value {
    match v {
        MysqlValue::NULL          => Value::Null,
        MysqlValue::Int(i)        => json!(i),
        MysqlValue::UInt(u)       => json!(u),
        MysqlValue::Float(f)      => json!(f as f64),
        MysqlValue::Double(f)     => json!(f),
        MysqlValue::Bytes(bytes) => {
            // Try to decode as UTF-8 first; fall back to lossy.
            let s = String::from_utf8(bytes)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
            // If the string looks like a plain integer (e.g. text-protocol int),
            // keep it as a string — callers can coerce if they need it as a
            // number. Simpler than guessing the column type.
            Value::String(s)
        }
        MysqlValue::Date(y, mo, d, h, mi, s, us) => {
            Value::String(format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{us:06}"))
        }
        MysqlValue::Time(neg, days, h, mi, s, us) => {
            let sign = if neg { "-" } else { "" };
            Value::String(format!("{sign}{days}d {h:02}:{mi:02}:{s:02}.{us:06}"))
        }
    }
}

/// Convert a `mysql_async::Row` to a `serde_json::Value` Object.
///
/// Column name → cell value. Column names come from `row.columns_ref()`.
/// Single-pass: binds `mut row` once, calls `take(i)` for each column in
/// order. No per-column clone of the whole row.
fn row_to_json(mut row: MysqlRow) -> Value {
    let columns = row.columns_ref().to_vec();
    let mut map = Map::new();
    for (i, col) in columns.iter().enumerate() {
        let name = col.name_str().into_owned();
        let cell: Option<MysqlValue> = row.take::<MysqlValue, usize>(i);
        map.insert(name, match cell {
            Some(v) => mysql_to_json(v),
            None    => Value::Null,
        });
    }
    Value::Object(map)
}

// ── DbClient ──────────────────────────────────────────────────────────────────

pub struct DbClient {
    pool: Pool,
}

impl DbClient {
    /// Build a new pool.  `host` is an IP or hostname; `port` is the TCP
    /// port (typically 3306).  TLS is OFF — the MySQL link is pod-local.
    pub fn new(host: &str, port: u16, user: &str, password: &str) -> Self {
        let opts: Opts = OptsBuilder::default()
            .ip_or_hostname(host)
            .tcp_port(port)
            .user(Some(user))
            .pass(Some(password))
            .into();
        Self { pool: Pool::new(opts) }
    }

    /// Execute a pre-registered template query.
    ///
    /// Returns each result row as a `serde_json::Value::Object` mapping
    /// column name → value.
    pub async fn query(
        &self,
        name: &str,
        params: &Value,
    ) -> Result<Vec<Value>, DbError> {
        let (tpl, ordered_params) = validate_template(name, params)?;

        let mut conn = self.pool.get_conn().await?;

        // Switch database.  Backtick-quote the identifier for defense-in-depth
        // (db names are a static allowlist, but quoting is the correct form).
        conn.query_drop(format!("USE `{}`", tpl.db)).await?;

        // Convert JSON params to mysql_async Params.
        let mysql_params: Vec<MysqlValue> = ordered_params
            .iter()
            .map(json_to_mysql)
            .collect();

        // Rewrite %s → ? for mysql_async prepared statements.
        let prepared_sql = tpl.sql.replace("%s", "?");

        let rows: Vec<MysqlRow> = conn
            .exec(prepared_sql, mysql_params)
            .await?;

        Ok(rows.into_iter().map(row_to_json).collect())
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── template registry ──────────────────────────────────────────────────

    #[test]
    fn all_6_templates_registered() {
        let names: Vec<&str> = V1_TEMPLATES.iter().map(|t| t.name).collect();
        assert_eq!(names.len(), 6, "expected 6 templates, got {}", names.len());
        assert!(names.contains(&"bracket_set_bonus_map_for"));
        assert!(names.contains(&"item_template_set_pieces"));
        assert!(names.contains(&"bracketsets_diag"));
        assert!(names.contains(&"character_online"));
        assert!(names.contains(&"character_online_by_class"));
        assert!(names.contains(&"game_event_all"));
    }

    #[test]
    fn all_templates_are_select() {
        for tpl in V1_TEMPLATES {
            let trimmed = tpl.sql.trim().to_uppercase();
            assert!(
                trimmed.starts_with("SELECT"),
                "{}: non-SELECT not allowed",
                tpl.name
            );
        }
    }

    #[test]
    fn no_string_interpolation_in_sql() {
        for tpl in V1_TEMPLATES {
            assert!(
                !tpl.sql.contains('{'),
                "{}: f-string-style placeholders forbidden",
                tpl.name
            );
            assert!(
                !tpl.sql.to_lowercase().contains("format"),
                "{}: .format() forbidden",
                tpl.name
            );
        }
    }

    #[test]
    fn all_templates_have_valid_db() {
        let valid_dbs = ["acore_world", "acore_characters", "acore_auth"];
        for tpl in V1_TEMPLATES {
            assert!(
                valid_dbs.contains(&tpl.db),
                "{}: unexpected db '{}'",
                tpl.name, tpl.db
            );
        }
    }

    #[test]
    fn bracket_set_bonus_map_signature() {
        let tpl = V1_TEMPLATES.iter().find(|t| t.name == "bracket_set_bonus_map_for").unwrap();
        assert_eq!(tpl.db, "acore_world");
        assert_eq!(tpl.params, &["itemset_id", "class_id", "spec_id"]);
        assert!(tpl.sql.contains("FROM bracket_set_bonus_map"));
    }

    #[test]
    fn game_event_all_is_parameter_free() {
        let tpl = V1_TEMPLATES.iter().find(|t| t.name == "game_event_all").unwrap();
        assert_eq!(tpl.db, "acore_world");
        assert!(tpl.params.is_empty(), "game_event_all must have no params");
        // Required columns per schema check 2026-05-30:
        assert!(tpl.sql.contains("eventEntry"));
        assert!(tpl.sql.contains("start_time"));
        assert!(tpl.sql.contains("end_time"));
        assert!(tpl.sql.contains("occurence")); // AC canonical misspelling
        assert!(tpl.sql.contains("length"));
        assert!(tpl.sql.contains("holiday"));
        assert!(tpl.sql.contains("holidayStage"));
        // Columns that must NOT appear (absent from this fork)
        assert!(!tpl.sql.to_lowercase().contains("state"), "state column does not exist");
        assert!(!tpl.sql.to_lowercase().contains("nextstart"), "nextstart column does not exist");
    }

    // ── validate_template (pure) ───────────────────────────────────────────

    #[test]
    fn unknown_template_returns_error() {
        let err = validate_template("no_such_template", &json!({})).unwrap_err();
        assert!(
            matches!(err, DbError::UnknownTemplate(ref s) if s == "no_such_template"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn missing_param_returns_bad_params() {
        // bracket_set_bonus_map_for requires itemset_id, class_id, spec_id
        let err = validate_template(
            "bracket_set_bonus_map_for",
            &json!({"itemset_id": 90101}),
        )
        .unwrap_err();
        assert!(
            matches!(err, DbError::BadParams(ref s) if s.contains("missing param 'class_id'")),
            "unexpected: {err}"
        );
    }

    #[test]
    fn validate_returns_ordered_params_for_bracket_set() {
        let (tpl, ordered) = validate_template(
            "bracket_set_bonus_map_for",
            &json!({"itemset_id": 90101, "class_id": 3, "spec_id": 2}),
        )
        .unwrap();
        assert!(tpl.sql.contains("FROM bracket_set_bonus_map"));
        assert_eq!(ordered.len(), 3);
        assert_eq!(ordered[0], json!(90101));
        assert_eq!(ordered[1], json!(3));
        assert_eq!(ordered[2], json!(2));
    }

    #[test]
    fn validate_game_event_all_no_params() {
        // game_event_all has empty params — passes with empty object
        let (tpl, ordered) = validate_template("game_event_all", &json!({})).unwrap();
        assert!(ordered.is_empty());
        assert_eq!(tpl.name, "game_event_all");
    }

    // ── json_to_mysql + mysql_to_json round-trip (pure) ────────────────────

    #[test]
    fn null_roundtrips() {
        let v = json_to_mysql(&Value::Null);
        assert_eq!(v, MysqlValue::NULL);
        assert_eq!(mysql_to_json(v), Value::Null);
    }

    #[test]
    fn int_roundtrips() {
        let v = json_to_mysql(&json!(42_i64));
        assert_eq!(v, MysqlValue::Int(42));
        assert_eq!(mysql_to_json(v), json!(42_i64));
    }

    #[test]
    fn string_roundtrips() {
        let v = json_to_mysql(&json!("hello"));
        assert_eq!(v, MysqlValue::Bytes(b"hello".to_vec()));
        assert_eq!(mysql_to_json(v), json!("hello"));
    }

    // ── live DB test (ignored by default) ─────────────────────────────────
    //
    // Run manually:
    //   AC_MYSQL_HOST=127.0.0.1 AC_MYSQL_PORT=3306 \
    //   AC_MYSQL_USER=root AC_MYSQL_PASS=root \
    //   cargo test -p harness-rs -- db_client::tests::live_query --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_query() {
        let host = std::env::var("AC_MYSQL_HOST").unwrap_or("127.0.0.1".into());
        let port: u16 = std::env::var("AC_MYSQL_PORT")
            .unwrap_or("3306".into())
            .parse()
            .unwrap();
        let user = std::env::var("AC_MYSQL_USER").unwrap_or("root".into());
        let pass = std::env::var("AC_MYSQL_PASS").unwrap_or("root".into());

        let client = DbClient::new(&host, port, &user, &pass);
        let rows = client
            .query("game_event_all", &json!({}))
            .await
            .expect("live query failed");
        println!("game_event_all: {} rows", rows.len());
        assert!(!rows.is_empty(), "expected at least one game event");
        // Each row must be an Object with eventEntry key
        assert!(rows[0].get("eventEntry").is_some());
    }
}
