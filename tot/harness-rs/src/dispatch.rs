//! Shared dispatch core for HTTP and MCP surfaces.
//!
//! Port of `harness_daemon/app.py:64-208` (`dispatch_tool` function).
//! Both transports call this; neither audit write nor transport-specific
//! envelope injection happens here — that is the caller's responsibility.

use serde_json::{json, Value};

use crate::ac_client::{ACClient, ACClientError};
use crate::auth::{find_matching_pattern, is_self_scope, AuthResult};
use crate::db_client::{DbClient, DbError};
use crate::registry::{Registry, ToolNotFound};

// ── DispatchOutcome ───────────────────────────────────────────────────────────

/// Result of `dispatch_tool`.
///
/// Both surfaces (HTTP, MCP) translate this into their transport's native
/// shape. The `body` shape MUST match what the V1.x HTTP endpoints emit
/// today (callers depend on it).
#[derive(Debug, Clone)]
pub struct DispatchOutcome {
    pub status:        u16,
    pub body:          Value,
    pub audit_outcome: String,
    pub error_detail:  String,
    pub ac_latency_ms: Option<u64>,
}

// ── dispatch_tool ─────────────────────────────────────────────────────────────

/// Shared dispatch logic.  Steps in order (mirrors app.py:64-208 EXACTLY):
///
/// 1. Scope match
/// 2. Registry lookup
/// 3. Self-binding (when matched pattern is `<ns>.self.*`)
/// 4. `gm.run_console` allowlist
/// 5. Daemon-direct (`obs.query_db`) OR forward to AC
///
/// Callers MUST have already authenticated the bearer and parsed `args`
/// as JSON. This function does NOT write to the audit log.
pub async fn dispatch_tool(
    name:       &str,
    args:       &Value,
    auth:       &AuthResult,
    request_id: &str,
    registry:   &Registry,
    ac_client:  &ACClient,
    db_client:  Option<&DbClient>,
) -> DispatchOutcome {
    // ── 1. Scope match ─────────────────────────────────────────────────────
    let matched_pattern = find_matching_pattern(&auth.scope, name);
    if matched_pattern.is_none() {
        return DispatchOutcome {
            status:        403,
            body:          json!({
                "ok":    false,
                "error": "scope_denied",
                "detail": "",
                "needed": name,
                "scope":  auth.scope,
            }),
            audit_outcome: "scope_denied".to_string(),
            error_detail:  String::new(),
            ac_latency_ms: None,
        };
    }
    let matched_pattern = matched_pattern.unwrap();

    // ── 2. Registry lookup ─────────────────────────────────────────────────
    let entry = match registry.find(name) {
        Ok(e)  => e,
        Err(ToolNotFound(_)) => {
            return DispatchOutcome {
                status:        404,
                body:          json!({"ok": false, "error": "unknown_tool", "detail": ""}),
                audit_outcome: "unknown_tool".to_string(),
                error_detail:  String::new(),
                ac_latency_ms: None,
            };
        }
    };

    // ── 3. Self-binding ────────────────────────────────────────────────────
    if is_self_scope(matched_pattern) {
        match auth.bound_to_guid {
            None => {
                let detail = "self-scope token has no bound_to_guid".to_string();
                return DispatchOutcome {
                    status:        403,
                    body:          json!({
                        "ok":     false,
                        "error":  "not_bound",
                        "detail": detail,
                    }),
                    audit_outcome: "not_bound".to_string(),
                    error_detail:  detail,
                    ac_latency_ms: None,
                };
            }
            Some(bound) => {
                if let Err(e) = entry.check_self_binding(args, bound) {
                    let detail = e.to_string();
                    return DispatchOutcome {
                        status:        403,
                        body:          json!({
                            "ok":     false,
                            "error":  "not_bound",
                            "detail": detail,
                        }),
                        audit_outcome: "not_bound".to_string(),
                        error_detail:  detail,
                        ac_latency_ms: None,
                    };
                }
            }
        }
    }

    // ── 4. gm.run_console allowlist ────────────────────────────────────────
    if name == "gm.run_console" {
        let cmd = args
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("");

        const ALLOWED_PREFIXES: &[&str] = &[".lookup", ".gobject", ".npc", ".bracketsets"];
        if !ALLOWED_PREFIXES.iter().any(|p| cmd.starts_with(p)) {
            // error_detail: cmd[:40] — use chars().take(40) for Unicode safety
            let cmd_truncated: String = cmd.chars().take(40).collect();
            let error_detail = format!("command not allowlisted: {cmd_truncated}");

            // body.detail: Python's tuple repr of ALLOWED_PREFIXES
            let body_detail =
                "run_console only accepts: ('.lookup', '.gobject', '.npc', '.bracketsets')";

            return DispatchOutcome {
                status:        403,
                body:          json!({
                    "ok":     false,
                    "error":  "scope_denied",
                    "detail": body_detail,
                }),
                audit_outcome: "scope_denied".to_string(),
                error_detail,
                ac_latency_ms: None,
            };
        }
    }

    // ── 5a. Daemon-direct path ─────────────────────────────────────────────
    if !entry.forwards_to_ac {
        if name != "obs.query_db" {
            return DispatchOutcome {
                status:        501,
                body:          json!({
                    "ok":     false,
                    "error":  "not_implemented",
                    "detail": format!("daemon-direct tool '{name}' not yet wired"),
                }),
                audit_outcome: "not_implemented".to_string(),
                error_detail:  String::new(),
                ac_latency_ms: None,
            };
        }

        let db = match db_client {
            Some(c) => c,
            None    => {
                let detail = "db_client not configured".to_string();
                return DispatchOutcome {
                    status:        503,
                    body:          json!({
                        "ok":     false,
                        "error":  "unavailable",
                        "detail": detail,
                    }),
                    audit_outcome: "db_not_configured".to_string(),
                    error_detail:  detail,
                    ac_latency_ms: None,
                };
            }
        };

        // template_name and params extracted from args (mirrors app.py:163-165)
        let template_name = args
            .get("template_name")
            .and_then(Value::as_str)
            .unwrap_or("");
        let tpl_params = args
            .get("params")
            .cloned()
            .unwrap_or_else(|| json!({}));

        return match db.query(template_name, &tpl_params).await {
            Err(DbError::UnknownTemplate(e)) => {
                let detail = format!("unknown template: {e}");
                DispatchOutcome {
                    status:        400,
                    body:          json!({"ok": false, "error": "bad_request", "detail": detail}),
                    audit_outcome: "bad_request".to_string(),
                    error_detail:  detail,
                    ac_latency_ms: None,
                }
            }
            Err(DbError::BadParams(e)) => {
                let detail = e.clone();
                DispatchOutcome {
                    status:        400,
                    body:          json!({"ok": false, "error": "bad_request", "detail": detail}),
                    audit_outcome: "bad_request".to_string(),
                    error_detail:  detail,
                    ac_latency_ms: None,
                }
            }
            Err(DbError::Mysql(e)) => {
                // Unexpected MySQL transport error — report as unavailable.
                let detail = e.to_string();
                DispatchOutcome {
                    status:        503,
                    body:          json!({"ok": false, "error": "unavailable", "detail": detail}),
                    audit_outcome: "db_error".to_string(),
                    error_detail:  detail,
                    ac_latency_ms: None,
                }
            }
            Ok(rows) => {
                let row_count = rows.len();
                DispatchOutcome {
                    status:        200,
                    body:          json!({"ok": true, "result": {"rows": rows, "row_count": row_count}}),
                    audit_outcome: "ok".to_string(),
                    error_detail:  String::new(),
                    ac_latency_ms: None,
                }
            }
        };
    }

    // ── 5b. Forward to AC ──────────────────────────────────────────────────
    let ac_resp = match ac_client
        .dispatch(name, args.clone(), request_id, &auth.identity)
        .await
    {
        Err(ACClientError::Transport(e)) => {
            let detail = e.to_string();
            return DispatchOutcome {
                status:        503,
                body:          json!({"ok": false, "error": "unavailable", "detail": detail}),
                audit_outcome: "ac_unreachable".to_string(),
                error_detail:  detail,
                ac_latency_ms: None,
            };
        }
        Ok(r) => r,
    };

    let outcome = if ac_resp.status == 200 { "ok" } else { "ac_error" };
    let error_detail = if ac_resp.status != 200 {
        ac_resp
            .body
            .get("detail")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };

    DispatchOutcome {
        status:        ac_resp.status,
        body:          ac_resp.body,
        audit_outcome: outcome.to_string(),
        error_detail,
        ac_latency_ms: Some(ac_resp.ac_latency_ms),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthResult;
    use crate::registry::build_v1_registry;
    use crate::test_support::{spawn_mock_ac, spawn_mock_ac_with_config, MockConfig};
    use serde_json::json;

    // ── builder helpers ────────────────────────────────────────────────────

    fn gm_auth() -> AuthResult {
        AuthResult {
            identity:      "gm.tbrack".to_string(),
            scope:         vec!["gm.*".to_string(), "obs.*".to_string()],
            bound_to_guid: None,
            augmented:     false,
        }
    }

    fn bot_auth(bound: Option<i64>) -> AuthResult {
        AuthResult {
            identity:      "bot.Krak".to_string(),
            scope:         vec!["bot.self.*".to_string(), "obs.self.*".to_string()],
            bound_to_guid: bound,
            augmented:     true,
        }
    }

    // ── 1. scope_denied ────────────────────────────────────────────────────

    #[tokio::test]
    async fn scope_denied_returns_403_with_needed_and_scope() {
        let mock   = spawn_mock_ac().await;
        let client = ACClient::new(mock.base_url(), 3.0);
        let reg    = build_v1_registry();

        // bot token has bot.self.* / obs.self.* — no access to gm.additem
        let out = dispatch_tool(
            "gm.additem",
            &json!({"target_guid": 42, "item_entry": 1}),
            &bot_auth(Some(42)),
            "req-1",
            &reg,
            &client,
            None,
        )
        .await;

        assert_eq!(out.status, 403);
        assert_eq!(out.body["error"], "scope_denied");
        assert_eq!(out.body["needed"], "gm.additem");
        assert_eq!(out.audit_outcome, "scope_denied");
        // scope field must be the token's scope list
        let scope_arr = out.body["scope"].as_array().unwrap();
        assert!(scope_arr.iter().any(|s| s == "bot.self.*"));
    }

    // ── 2. unknown_tool ────────────────────────────────────────────────────

    #[tokio::test]
    async fn unknown_tool_returns_404() {
        let mock   = spawn_mock_ac().await;
        let client = ACClient::new(mock.base_url(), 3.0);
        let reg    = build_v1_registry();

        let out = dispatch_tool(
            "gm.does_not_exist",
            &json!({}),
            &gm_auth(),
            "req-2",
            &reg,
            &client,
            None,
        )
        .await;

        assert_eq!(out.status, 404);
        assert_eq!(out.body["error"], "unknown_tool");
        assert_eq!(out.audit_outcome, "unknown_tool");
    }

    // ── 3. not_bound — no bound_to_guid ────────────────────────────────────

    #[tokio::test]
    async fn not_bound_no_guid_returns_403() {
        let mock   = spawn_mock_ac().await;
        let client = ACClient::new(mock.base_url(), 3.0);
        let reg    = build_v1_registry();

        // bot.self.* token but no bound_to_guid
        let out = dispatch_tool(
            "bot.set_goal",
            &json!({"bot_guid": 42}),
            &bot_auth(None),
            "req-3",
            &reg,
            &client,
            None,
        )
        .await;

        assert_eq!(out.status, 403);
        assert_eq!(out.body["error"], "not_bound");
        assert!(
            out.body["detail"]
                .as_str()
                .unwrap_or("")
                .contains("self-scope token has no bound_to_guid"),
            "unexpected detail: {}",
            out.body["detail"]
        );
        assert_eq!(out.audit_outcome, "not_bound");
    }

    // ── 3. not_bound — subject mismatch ────────────────────────────────────

    #[tokio::test]
    async fn not_bound_subject_mismatch_returns_403() {
        let mock   = spawn_mock_ac().await;
        let client = ACClient::new(mock.base_url(), 3.0);
        let reg    = build_v1_registry();

        // bound to 42 but calling with bot_guid=999
        let out = dispatch_tool(
            "bot.set_goal",
            &json!({"bot_guid": 999}),
            &bot_auth(Some(42)),
            "req-4",
            &reg,
            &client,
            None,
        )
        .await;

        assert_eq!(out.status, 403);
        assert_eq!(out.body["error"], "not_bound");
        assert_eq!(out.audit_outcome, "not_bound");
        // Detail must mention the mismatch
        let detail = out.body["detail"].as_str().unwrap_or("");
        assert!(
            detail.contains("not_bound") || detail.contains("999") || detail.contains("42"),
            "expected mismatch info in detail: {detail}"
        );
    }

    // ── 4. run_console non-allowlisted ─────────────────────────────────────

    #[tokio::test]
    async fn run_console_non_allowlisted_command_returns_403() {
        let mock   = spawn_mock_ac().await;
        let client = ACClient::new(mock.base_url(), 3.0);
        let reg    = build_v1_registry();

        let out = dispatch_tool(
            "gm.run_console",
            &json!({"command": ".ban account alice"}),
            &gm_auth(),
            "req-5",
            &reg,
            &client,
            None,
        )
        .await;

        assert_eq!(out.status, 403);
        assert_eq!(out.body["error"], "scope_denied");
        assert_eq!(out.audit_outcome, "scope_denied");
        // EXACT body.detail string — Python's tuple repr
        assert_eq!(
            out.body["detail"].as_str().unwrap_or(""),
            "run_console only accepts: ('.lookup', '.gobject', '.npc', '.bracketsets')"
        );
        // error_detail contains the truncated command
        assert!(out.error_detail.contains("command not allowlisted"));
    }

    // ── 5. obs.query_db without db_client → 503 ────────────────────────────

    #[tokio::test]
    async fn query_db_without_db_client_returns_503() {
        let mock   = spawn_mock_ac().await;
        let client = ACClient::new(mock.base_url(), 3.0);
        // Add obs.query_db to the token scope
        let auth = AuthResult {
            identity:      "obs.agent".to_string(),
            scope:         vec!["obs.*".to_string()],
            bound_to_guid: None,
            augmented:     false,
        };
        let reg = build_v1_registry();

        let out = dispatch_tool(
            "obs.query_db",
            &json!({"template_name": "game_event_all", "params": {}}),
            &auth,
            "req-6",
            &reg,
            &client,
            None, // db_client is None
        )
        .await;

        assert_eq!(out.status, 503);
        assert_eq!(out.body["error"], "unavailable");
        assert_eq!(out.audit_outcome, "db_not_configured");
        assert_eq!(out.error_detail, "db_client not configured");
    }

    // ── 6. forward ok → status 200, ac_latency_ms Some, audit "ok" ────────

    #[tokio::test]
    async fn forward_ok_returns_ac_response() {
        let mock = spawn_mock_ac_with_config(MockConfig {
            dispatch_status: 200,
            dispatch_body:   json!({"ok": true, "result": {"pong": true, "ts_ms": 1234}}),
            health_status:   200,
        })
        .await;
        let client = ACClient::new(mock.base_url(), 3.0);
        let reg    = build_v1_registry();

        let out = dispatch_tool(
            "obs.ping",
            &json!({}),
            &gm_auth(),
            "req-7",
            &reg,
            &client,
            None,
        )
        .await;

        assert_eq!(out.status, 200);
        assert_eq!(out.body["ok"], true);
        assert_eq!(out.body["result"]["pong"], true);
        assert_eq!(out.audit_outcome, "ok");
        assert!(out.ac_latency_ms.is_some(), "ac_latency_ms must be Some");
        assert!(out.error_detail.is_empty());
    }

    // ── 7. forward ac_error → status preserved, audit "ac_error" ──────────

    #[tokio::test]
    async fn forward_ac_error_propagates_status() {
        let mock = spawn_mock_ac_with_config(MockConfig {
            dispatch_status: 400,
            dispatch_body:   json!({"ok": false, "error": "bridge_validation", "detail": "bad arg"}),
            health_status:   200,
        })
        .await;
        let client = ACClient::new(mock.base_url(), 3.0);
        let reg    = build_v1_registry();

        let out = dispatch_tool(
            "obs.ping",
            &json!({}),
            &gm_auth(),
            "req-8",
            &reg,
            &client,
            None,
        )
        .await;

        assert_eq!(out.status, 400);
        assert_eq!(out.body["error"], "bridge_validation");
        assert_eq!(out.audit_outcome, "ac_error");
        assert_eq!(out.error_detail, "bad arg");
        assert!(out.ac_latency_ms.is_some());
    }

    // ── 8. ac_unreachable → 503, audit "ac_unreachable" ───────────────────

    #[tokio::test]
    async fn ac_unreachable_returns_503() {
        let client = ACClient::new("http://127.0.0.1:1", 1.0);
        let reg    = build_v1_registry();

        let out = dispatch_tool(
            "obs.ping",
            &json!({}),
            &gm_auth(),
            "req-9",
            &reg,
            &client,
            None,
        )
        .await;

        assert_eq!(out.status, 503);
        assert_eq!(out.body["error"], "unavailable");
        assert_eq!(out.audit_outcome, "ac_unreachable");
        assert!(!out.error_detail.is_empty());
    }

    // ── run_console allowlisted command passes through ─────────────────────

    #[tokio::test]
    async fn run_console_allowlisted_command_forwarded() {
        let mock = spawn_mock_ac_with_config(MockConfig {
            dispatch_status: 200,
            dispatch_body:   json!({"ok": true, "result": {"output": "found item"}}),
            health_status:   200,
        })
        .await;
        let client = ACClient::new(mock.base_url(), 3.0);
        let reg    = build_v1_registry();

        let out = dispatch_tool(
            "gm.run_console",
            &json!({"command": ".lookup item 19019"}),
            &gm_auth(),
            "req-10",
            &reg,
            &client,
            None,
        )
        .await;

        assert_eq!(out.status, 200);
        assert_eq!(out.audit_outcome, "ok");
    }
}
