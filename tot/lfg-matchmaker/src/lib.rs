//! `lfg-matchmaker` library crate.
//!
//! Exposes the matchmaker's public API for consumption by `slice-host` and any
//! future host binary.  The binary entry point (`main.rs`) uses these same
//! exports so all logic lives here.
//!
//! # Public surface
//!
//! - [`api::AppState`], [`api::routes`] — axum router (`/queue`); the host owns
//!   `/healthz` and mounts these under a prefix.
//! - [`tick::run`] — the perpetual tick loop; spawn in a supervised tokio task.
//! - [`config::Config`] — reads `HARNESS_BASE_URL`, `HARNESS_BEARER`,
//!   `LFG_*` env vars.  `LFG_ENABLED` defaults `false` (inert/shadow mode).
//! - [`harness::Harness`] — `{ok,result}` HTTP client for the harness wire.

pub mod api;
pub mod config;
pub mod harness;
pub mod matcher;
pub mod orchestrator;
pub mod queue;
pub mod roster;
pub mod tick;
pub mod types;
