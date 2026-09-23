//! Typed event bus. Plugins communicate through namespaced events
//! (`namespace.event`) rather than direct imports. Events are notifications,
//! not RPC: a sender MUST NOT assume a receiver exists (spec §28-29).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Namespaced name, e.g. `memo.created`.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub data: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Validates event naming: `namespace.event`, lowercase, dots allowed.
pub fn valid_event_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 128 {
        return false;
    }
    let mut parts = 0;
    for seg in name.split('.') {
        if seg.is_empty() {
            return false;
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return false;
        }
        parts += 1;
    }
    parts >= 2
}

struct Subscriber {
    id: u64,
    /// `None` receives all events; `Some(name)` receives exact matches only.
    filter: Option<String>,
    sender: mpsc::UnboundedSender<Event>,
}

#[derive(Default)]
pub struct EventBusState {
    next_id: AtomicU64,
    subscribers: Mutex<Vec<Subscriber>>,
}

#[derive(Clone, Default)]
pub struct EventBus {
    state: Arc<EventBusState>,
}

/// A live subscription; dropping it unsubscribes.
pub struct Subscription {
    bus: EventBus,
    id: u64,
    pub receiver: mpsc::UnboundedReceiver<Event>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.bus
            .state
            .subscribers
            .lock()
            .unwrap()
            .retain(|s| s.id != self.id);
    }
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self, filter: Option<String>) -> Subscription {
        let id = self.state.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        self.state.subscribers.lock().unwrap().push(Subscriber {
            id,
            filter,
            sender: tx,
        });
        Subscription {
            bus: self.clone(),
            id,
            receiver: rx,
        }
    }

    /// Emit an event to all matching subscribers. Returns delivery count.
    /// Emitters never block and never fail when nobody listens.
    pub fn emit(&self, name: &str, source: Option<String>, data: serde_json::Value) -> usize {
        if !valid_event_name(name) {
            tracing::warn!(event = name, "refusing to emit event with invalid name");
            return 0;
        }
        let event = Event {
            name: name.to_string(),
            source,
            data,
            timestamp: chrono::Utc::now(),
        };
        let mut delivered = 0;
        let mut dead = Vec::new();
        let subs = self.state.subscribers.lock().unwrap();
        for (idx, sub) in subs.iter().enumerate() {
            let matches = sub.filter.as_deref().map(|f| f == name).unwrap_or(true);
            if matches {
                if sub.sender.send(event.clone()).is_ok() {
                    delivered += 1;
                } else {
                    dead.push(idx);
                }
            }
        }
        drop(subs);
        if !dead.is_empty() {
            let mut subs = self.state.subscribers.lock().unwrap();
            for idx in dead.into_iter().rev() {
                if idx < subs.len() {
                    subs.remove(idx);
                }
            }
        }
        delivered
    }

    pub fn subscriber_count(&self) -> usize {
        self.state.subscribers.lock().unwrap().len()
    }
}

/// Helper for building payload maps.
pub fn payload(entries: HashMap<&str, serde_json::Value>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (k, v) in entries {
        map.insert(k.to_string(), v);
    }
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn emit_subscribe_roundtrip() {
        let bus = EventBus::new();
        let mut sub = bus.subscribe(Some("memo.created".into()));
        let delivered = bus.emit(
            "memo.created",
            Some("eigendesk.memo".into()),
            serde_json::json!({"uri": "memo://1"}),
        );
        assert_eq!(delivered, 1);
        let ev = sub.receiver.recv().await.unwrap();
        assert_eq!(ev.name, "memo.created");
        assert_eq!(ev.data["uri"], "memo://1");
    }

    #[tokio::test]
    async fn no_receivers_is_fine() {
        let bus = EventBus::new();
        assert_eq!(bus.emit("memo.created", None, serde_json::json!({})), 0);
    }

    #[tokio::test]
    async fn wildcard_receives_all() {
        let bus = EventBus::new();
        let mut sub = bus.subscribe(None);
        bus.emit("memo.created", None, serde_json::json!({}));
        bus.emit("task.completed", None, serde_json::json!({}));
        assert!(sub.receiver.recv().await.is_some());
        assert!(sub.receiver.recv().await.is_some());
    }

    #[test]
    fn drop_unsubscribes() {
        let bus = EventBus::new();
        {
            let _sub = bus.subscribe(None);
            assert_eq!(bus.subscriber_count(), 1);
        }
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn event_name_validation() {
        assert!(valid_event_name("memo.created"));
        assert!(valid_event_name("slurm.job-finished"));
        assert!(!valid_event_name("invalid"));
        assert!(!valid_event_name(".leading"));
        assert!(!valid_event_name("UPPER.case"));
        assert!(!valid_event_name(""));
    }
}
