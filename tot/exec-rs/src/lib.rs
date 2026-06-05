//! `exec-rs` — mechanical execution layer for the LLM-native bot system.
//!
//! Receives goals from `brain-rs` and disposes them via the harness wire.
//! This crate owns navigation, movement, and future rotation/ability execution.
//!
//! # Public surface
//!
//! - [`nav`] — navmesh client + movement orchestrator: [`nav::find_path`],
//!   [`nav::move_path`], [`nav::walk_to`], typed [`nav::Path`], [`nav::MoveResult`], [`nav::flags`].

pub mod nav;
