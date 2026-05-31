//! `game-event-scheduler` library crate.
//!
//! Exposes the scheduler's public API for consumption by `slice-host` and any future
//! host binary.  The binary entry point (`main.rs`) uses these same exports so all
//! logic lives here.
//!
//! # Public surface
//!
//! - [`api::AppState`], [`api::routes`] — axum router (`/report`, `/report/history`);
//!   the host owns `/healthz` and mounts these under a prefix.
//! - [`tick::run`] — the perpetual tick loop; spawn in a supervised tokio task.
//! - [`config::Config`] — reads `HARNESS_BASE_URL`, `HARNESS_BEARER`, `GES_*` env vars.
//! - [`harness::Harness`] — `{ok,result}` HTTP client for the harness wire.

pub mod api;
pub mod config;
pub mod events;
pub mod harness;
pub mod holiday;
pub mod packed;
pub mod resolve;
pub mod schedule;
pub mod shadow;
pub mod tick;
