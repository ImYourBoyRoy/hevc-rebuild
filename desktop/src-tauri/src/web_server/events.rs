//! In-process event bus behind `GET /api/events` (Server-Sent Events). Events carry the same names and
//! JSON payloads the desktop window receives, plus a monotonically increasing id so a reconnecting
//! browser resumes with `Last-Event-ID` instead of losing run lines.

use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

const BACKLOG: usize = 1000;
/// Output lines kept per run for a tab that opens mid-run.
const REPLAY_LINES: usize = 240;
const CHANNEL: usize = 1024;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ServerEvent {
    pub(crate) id: u64,
    pub(crate) event: String,
    pub(crate) payload: Value,
}

#[derive(Clone)]
pub(crate) struct EventBus {
    inner: Arc<Mutex<Inner>>,
}

/// What a window that opens during a run needs to catch up: the newest lines, the latest progress and
/// resource sample. Dropped when the run finishes.
#[derive(Default)]
struct RunReplay {
    lines: VecDeque<(String, Value)>,
    progress: Option<Value>,
    resources: Option<Value>,
}

struct Inner {
    replay: HashMap<String, RunReplay>,
    next_id: u64,
    backlog: VecDeque<ServerEvent>,
    sender: broadcast::Sender<ServerEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        let (sender, _) = broadcast::channel(CHANNEL);
        EventBus {
            inner: Arc::new(Mutex::new(Inner {
                replay: HashMap::new(),
                next_id: 1,
                backlog: VecDeque::new(),
                sender,
            })),
        }
    }
}

impl EventBus {
    pub(crate) fn publish(&self, event: &str, payload: Value) -> u64 {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let id = inner.next_id;
        inner.next_id += 1;
        let run_key = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_string);
        match event {
            "run-line" | "repair-line" => {
                if let Some(run) = run_key("runId") {
                    let replay = inner.replay.entry(run).or_default();
                    replay.lines.push_back((event.to_string(), payload.clone()));
                    while replay.lines.len() > REPLAY_LINES {
                        replay.lines.pop_front();
                    }
                }
            }
            "run-progress" => {
                if let Some(run) = run_key("runId") {
                    inner.replay.entry(run).or_default().progress = Some(payload.clone());
                }
            }
            "run-resources" => {
                if let Some(run) = run_key("runId") {
                    inner.replay.entry(run).or_default().resources = Some(payload.clone());
                }
            }
            "run-finished" => {
                if let Some(run) = run_key("runId") {
                    inner.replay.remove(&run);
                }
            }
            "repair-finished" => {
                if let Some(run) = run_key("jobId") {
                    inner.replay.remove(&run);
                }
            }
            _ => {}
        }
        let message = ServerEvent {
            id,
            event: event.to_string(),
            payload,
        };
        inner.backlog.push_back(message.clone());
        while inner.backlog.len() > BACKLOG {
            inner.backlog.pop_front();
        }
        let _ = inner.sender.send(message);
        id
    }

    /// A new window (no `Last-Event-ID`) during a run: a synthetic `job-started` for the active job, then
    /// its recent lines and latest progress, then the live stream. Taken under one lock, so nothing falls
    /// between the replay and what follows. The replay events carry no ids (they are not in the backlog).
    pub(crate) fn subscribe_active(
        &self,
        active: Option<(String, String)>,
    ) -> (Vec<(String, Value)>, broadcast::Receiver<ServerEvent>) {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let receiver = inner.sender.subscribe();
        let mut replay = Vec::new();
        if let Some((run, kind)) = active {
            replay.push((
                "job-started".to_string(),
                serde_json::json!({"runId": run, "kind": kind}),
            ));
            if let Some(saved) = inner.replay.get(&run) {
                replay.extend(saved.lines.iter().cloned());
                replay.extend(
                    saved
                        .progress
                        .iter()
                        .map(|payload| ("run-progress".to_string(), payload.clone())),
                );
                replay.extend(
                    saved
                        .resources
                        .iter()
                        .map(|payload| ("run-resources".to_string(), payload.clone())),
                );
            }
        }
        (replay, receiver)
    }

    /// Everything after `last_id` that is still remembered, plus a receiver for what follows. Both are
    /// taken under one lock, so no event can fall between them.
    pub(crate) fn subscribe_since(
        &self,
        last_id: Option<u64>,
    ) -> (Vec<ServerEvent>, broadcast::Receiver<ServerEvent>) {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let receiver = inner.sender.subscribe();
        let missed = match last_id {
            Some(last) => inner
                .backlog
                .iter()
                .filter(|event| event.id > last)
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        (missed, receiver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn subscribers_receive_events_in_order_with_increasing_ids() {
        let bus = EventBus::default();
        let (_, mut receiver) = bus.subscribe_since(None);
        let first = bus.publish("run-line", json!({"runId": "a", "line": "one"}));
        let second = bus.publish("run-finished", json!({"runId": "a"}));
        assert!(second > first);
        let a = receiver.recv().await.unwrap();
        let b = receiver.recv().await.unwrap();
        assert_eq!((a.id, a.event.as_str()), (first, "run-line"));
        assert_eq!((b.id, b.event.as_str()), (second, "run-finished"));
        assert_eq!(a.payload["line"], "one");
    }

    #[tokio::test]
    async fn two_subscribers_both_see_the_same_job() {
        let bus = EventBus::default();
        let (_, mut browser) = bus.subscribe_since(None);
        let (_, mut agent) = bus.subscribe_since(None);
        bus.publish("run-progress", json!({"runId": "x"}));
        assert_eq!(browser.recv().await.unwrap(), agent.recv().await.unwrap());
    }

    #[test]
    fn reconnect_replays_only_events_after_the_last_seen_id() {
        let bus = EventBus::default();
        let one = bus.publish("run-line", json!(1));
        let two = bus.publish("run-line", json!(2));
        let three = bus.publish("run-line", json!(3));
        let (missed, _) = bus.subscribe_since(Some(one));
        assert_eq!(
            missed.iter().map(|event| event.id).collect::<Vec<_>>(),
            vec![two, three]
        );
        let (nothing, _) = bus.subscribe_since(None);
        assert!(
            nothing.is_empty(),
            "a fresh subscriber starts at the live edge"
        );
        let (caught_up, _) = bus.subscribe_since(Some(three));
        assert!(caught_up.is_empty());
    }

    fn lines(events: &[(String, Value)]) -> Vec<String> {
        events
            .iter()
            .filter(|(name, _)| name == "run-line")
            .map(|(_, payload)| payload["line"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn a_late_subscriber_gets_the_active_runs_recent_events_without_a_last_event_id() {
        let bus = EventBus::default();
        bus.publish("run-line", json!({"runId": "run-1", "line": "one"}));
        bus.publish("run-line", json!({"runId": "other", "line": "not mine"}));
        bus.publish("run-line", json!({"runId": "run-1", "line": "two"}));
        bus.publish(
            "run-progress",
            json!({"runId": "run-1", "filePercent": 10.0}),
        );
        bus.publish(
            "run-progress",
            json!({"runId": "run-1", "filePercent": 55.0}),
        );
        bus.publish(
            "run-resources",
            json!({"runId": "run-1", "snapshot": {"cpu": 1}}),
        );
        let (replay, mut live) = bus.subscribe_active(Some(("run-1".into(), "media_run".into())));
        assert_eq!(
            replay[0],
            (
                "job-started".to_string(),
                json!({"runId": "run-1", "kind": "media_run"})
            ),
            "the window learns the run first"
        );
        assert_eq!(lines(&replay), ["one", "two"]);
        let progress: Vec<_> = replay
            .iter()
            .filter(|(name, _)| name == "run-progress")
            .collect();
        assert_eq!(progress.len(), 1, "only the latest progress");
        assert_eq!(progress[0].1["filePercent"], 55.0);
        assert!(replay.iter().any(|(name, _)| name == "run-resources"));
        bus.publish("run-line", json!({"runId": "run-1", "line": "three"}));
        assert_eq!(
            live.try_recv().unwrap().payload["line"],
            "three",
            "later events arrive live, with none lost or repeated"
        );
    }

    #[test]
    fn no_active_run_means_no_replay_and_a_finished_run_forgets_its_lines() {
        let bus = EventBus::default();
        bus.publish("run-line", json!({"runId": "run-1", "line": "one"}));
        let (none, _) = bus.subscribe_active(None);
        assert!(none.is_empty());
        bus.publish("run-finished", json!({"runId": "run-1"}));
        let (after, _) = bus.subscribe_active(Some(("run-1".into(), "media_run".into())));
        assert_eq!(
            after.len(),
            1,
            "only the synthetic job-started remains: {after:?}"
        );
        bus.publish("repair-line", json!({"runId": "fix-1", "line": "step"}));
        bus.publish(
            "repair-finished",
            json!({"jobId": "fix-1", "success": true}),
        );
        let (repair, _) = bus.subscribe_active(Some(("fix-1".into(), "system_provision".into())));
        assert_eq!(repair.len(), 1);
    }

    #[test]
    fn the_replay_keeps_only_the_newest_lines() {
        let bus = EventBus::default();
        for index in 0..(REPLAY_LINES + 30) {
            bus.publish(
                "run-line",
                json!({"runId": "run-1", "line": format!("l{index}")}),
            );
        }
        let (replay, _) = bus.subscribe_active(Some(("run-1".into(), "media_run".into())));
        let seen = lines(&replay);
        assert_eq!(seen.len(), REPLAY_LINES);
        assert_eq!(seen.first().unwrap(), "l30");
        assert_eq!(seen.last().unwrap(), &format!("l{}", REPLAY_LINES + 29));
    }

    #[test]
    fn backlog_is_bounded() {
        let bus = EventBus::default();
        for index in 0..(BACKLOG + 50) {
            bus.publish("run-line", json!(index));
        }
        let (missed, _) = bus.subscribe_since(Some(0));
        assert_eq!(missed.len(), BACKLOG);
        assert_eq!(missed[0].id, 51);
    }
}
