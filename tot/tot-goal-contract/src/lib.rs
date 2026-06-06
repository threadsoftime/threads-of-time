//! `tot-goal-contract` — the brain↔exec goal contract for the LLM-native bot system.
//!
//! Owns the `Goal` intent the brain emits, the `GoalStatus`/`EscalationEvent` the
//! executor returns, and the M1 in-process wire (`GoalSink`/`StatusSource` over tokio
//! channels). Both `brain-rs` and `exec-rs` depend on this crate so the two sides share
//! a compile-time-checked type. No dependency on other tot crates (no cycle).

pub mod goal;
pub mod status;
pub mod wire;

pub use goal::{Goal, GoalEnvelope, GrindGoal, MobFilter, WorldPos};
// pub use status::{BlockedReason, EscalationEvent, GoalProgress, GoalStatus};  // enabled in Task 3
// pub use wire::{wire, ChannelGoalSink, ChannelStatusSource, GoalSink, StatusSource};  // enabled in Task 5

/// The contract version baked into every `GoalEnvelope`. The executor rejects an
/// envelope whose `version` exceeds this. Bump only on a field change that alters
/// executor behavior — adding a new `Goal` variant does NOT require a bump.
pub const GOAL_CONTRACT_VERSION: u32 = 1;
