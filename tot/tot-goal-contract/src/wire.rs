//! The brain↔exec wire.
//!
//! M1 runs exec-rs in-process with brain-rs; the wire is a tokio `watch` channel
//! (latest-goal-wins) for brain→exec and an `mpsc` channel for exec→brain status.
//! The brain side calls the `GoalSink`/`StatusSource` traits; at M2 the same traits
//! are re-implemented over HTTP and exec splits into its own service — the brain call
//! sites do not change.

use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::{mpsc, watch, Mutex};

use crate::goal::GoalEnvelope;
use crate::status::GoalStatus;

/// brain → exec: set the current goal for a bot.
#[async_trait]
pub trait GoalSink: Send + Sync {
    async fn set_goal(&self, bot_guid: u64, envelope: GoalEnvelope) -> anyhow::Result<()>;
}

/// exec → brain: drain the latest status for a bot, if any (non-blocking).
#[async_trait]
pub trait StatusSource: Send + Sync {
    async fn poll_status(&self, bot_guid: u64) -> anyhow::Result<Option<GoalStatus>>;
}

/// The exec-side receiver of the current goal (latest-goal-wins).
pub type GoalReceiver = watch::Receiver<Option<GoalEnvelope>>;
/// The exec-side sender of status updates.
pub type StatusSender = mpsc::Sender<GoalStatus>;

const STATUS_CHANNEL_DEPTH: usize = 32;

/// In-process `GoalSink` over a `watch` channel, bound to one bot.
pub struct ChannelGoalSink {
    bot_guid: u64,
    tx: watch::Sender<Option<GoalEnvelope>>,
}

#[async_trait]
impl GoalSink for ChannelGoalSink {
    async fn set_goal(&self, bot_guid: u64, envelope: GoalEnvelope) -> anyhow::Result<()> {
        if bot_guid != self.bot_guid {
            anyhow::bail!("ChannelGoalSink bound to bot {} got bot {}", self.bot_guid, bot_guid);
        }
        envelope.validate()?;
        // watch::Sender::send errors only if all receivers dropped (exec gone) — surface it.
        self.tx.send(Some(envelope)).map_err(|e| anyhow::anyhow!("exec receiver gone: {e}"))?;
        Ok(())
    }
}

/// In-process `StatusSource` over an `mpsc` channel, bound to one bot.
pub struct ChannelStatusSource {
    bot_guid: u64,
    rx: Mutex<mpsc::Receiver<GoalStatus>>,
}

#[async_trait]
impl StatusSource for ChannelStatusSource {
    async fn poll_status(&self, bot_guid: u64) -> anyhow::Result<Option<GoalStatus>> {
        if bot_guid != self.bot_guid {
            anyhow::bail!("ChannelStatusSource bound to bot {} got bot {}", self.bot_guid, bot_guid);
        }
        Ok(self.rx.lock().await.try_recv().ok())
    }
}

/// A multiplexing [`GoalSink`] over many per-bot [`ChannelGoalSink`]s.
///
/// `set_goal(bot_guid, ..)` is routed to the sink registered for `bot_guid`; an
/// unregistered bot is an error (the brain emitted a goal for a bot whose exec
/// loop is not running). This lets `loop_supervisor`'s single
/// `goal_sink: Option<Arc<dyn GoalSink>>` field drive an arbitrary cohort with
/// no tick-path change.
#[derive(Default)]
pub struct GoalSinkRegistry {
    sinks: HashMap<u64, ChannelGoalSink>,
}

impl GoalSinkRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the per-bot sink for `bot_guid`. A re-registration overwrites
    /// (last wins) and returns the previous sink, if any.
    pub fn register(&mut self, bot_guid: u64, sink: ChannelGoalSink) -> Option<ChannelGoalSink> {
        self.sinks.insert(bot_guid, sink)
    }

    /// Number of enrolled bots.
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    /// True when no bots are enrolled.
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }
}

#[async_trait]
impl GoalSink for GoalSinkRegistry {
    async fn set_goal(&self, bot_guid: u64, envelope: GoalEnvelope) -> anyhow::Result<()> {
        match self.sinks.get(&bot_guid) {
            Some(sink) => sink.set_goal(bot_guid, envelope).await,
            None => anyhow::bail!("GoalSinkRegistry has no sink for bot {bot_guid}"),
        }
    }
}

/// Build an in-process wire for one bot. Returns the brain-side `(sink, source)` and
/// the exec-side `(goal_rx, status_tx)`. Dropping the returned `sink` closes the goal
/// channel, which signals shutdown to the exec loop.
///
/// Back-pressure: the status channel is bounded (`STATUS_CHANNEL_DEPTH`); if exec emits
/// status faster than brain polls it, `StatusSender::send` returns an error when full.
#[must_use]
pub fn wire(bot_guid: u64) -> (ChannelGoalSink, ChannelStatusSource, GoalReceiver, StatusSender) {
    let (goal_tx, goal_rx) = watch::channel(None);
    let (status_tx, status_rx) = mpsc::channel(STATUS_CHANNEL_DEPTH);
    (
        ChannelGoalSink { bot_guid, tx: goal_tx },
        ChannelStatusSource { bot_guid, rx: Mutex::new(status_rx) },
        goal_rx,
        status_tx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::{Goal, GoalEnvelope, GrindGoal, MobFilter, WorldPos};
    use crate::status::GoalStatus;

    fn env(id: &str) -> GoalEnvelope {
        GoalEnvelope {
            goal_id: id.into(),
            version: 1,
            goal: Goal::Grind(GrindGoal {
                anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
                wander_radius: 90.0, max_search_radius: 35.0,
                mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
                to_level: 6, kill_count: None, rest_threshold: 0.35,
            }),
        }
    }

    #[tokio::test]
    async fn pushed_goal_is_observed_by_exec_receiver() {
        let (sink, _source, mut goal_rx, _status_tx) = wire(1003);
        assert!(goal_rx.borrow().is_none(), "no goal before first push");
        sink.set_goal(1003, env("g-1")).await.unwrap();
        goal_rx.changed().await.unwrap();
        assert_eq!(goal_rx.borrow().as_ref().unwrap().goal_id, "g-1");
    }

    #[tokio::test]
    async fn latest_goal_wins() {
        let (sink, _source, mut goal_rx, _status_tx) = wire(1003);
        sink.set_goal(1003, env("g-1")).await.unwrap();
        sink.set_goal(1003, env("g-2")).await.unwrap();
        goal_rx.changed().await.unwrap();
        assert_eq!(goal_rx.borrow().as_ref().unwrap().goal_id, "g-2", "watch keeps only the latest");
    }

    #[tokio::test]
    async fn status_sent_by_exec_is_polled_by_brain() {
        let (_sink, source, _goal_rx, status_tx) = wire(1003);
        assert!(source.poll_status(1003).await.unwrap().is_none(), "nothing yet");
        status_tx.send(GoalStatus::Completed { summary: "done".into() }).await.unwrap();
        let got = source.poll_status(1003).await.unwrap();
        assert!(matches!(got, Some(GoalStatus::Completed { .. })));
    }

    #[tokio::test]
    async fn wrong_bot_guid_is_rejected() {
        let (sink, _source, _goal_rx, _status_tx) = wire(1003);
        let err = sink.set_goal(9999, env("g-1")).await;
        assert!(err.is_err(), "a channel bound to bot 1003 must reject bot 9999");
    }

    #[tokio::test]
    async fn registry_routes_to_the_registered_bot() {
        // Two bots, two channels, one registry.
        let (sink_a, _src_a, mut rx_a, _tx_a) = wire(1001);
        let (sink_b, _src_b, mut rx_b, _tx_b) = wire(1002);
        let mut reg = GoalSinkRegistry::new();
        reg.register(1001, sink_a);
        reg.register(1002, sink_b);
        assert_eq!(reg.len(), 2);

        // A goal for 1002 must land on 1002's receiver, not 1001's.
        reg.set_goal(1002, env("g-b")).await.unwrap();
        rx_b.changed().await.unwrap();
        assert_eq!(rx_b.borrow().as_ref().unwrap().goal_id, "g-b");
        assert!(rx_a.borrow().is_none(), "1001 must not receive 1002's goal");
    }

    #[tokio::test]
    async fn registry_rejects_unenrolled_bot() {
        let (sink_a, _src_a, _rx_a, _tx_a) = wire(1001);
        let mut reg = GoalSinkRegistry::new();
        reg.register(1001, sink_a);
        let err = reg.set_goal(9999, env("g-x")).await;
        assert!(err.is_err(), "a registry without bot 9999 must reject it");
    }

    #[tokio::test]
    async fn registry_is_empty_by_default() {
        let reg = GoalSinkRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }
}
