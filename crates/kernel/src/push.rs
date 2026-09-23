//! Push hub: batches kernel→UI messages and flushes them through the
//! installed sink (the Tauri shell evals them into the main frame).
//! Plugin-addressed messages (`plugin-push`) are only ever delivered to the
//! main frame, which routes them to the correct isolated iframe — plugin
//! iframes have no direct channel to the kernel.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{PushMessage, PushSink};

const FLUSH_INTERVAL: Duration = Duration::from_millis(12);
const MAX_BATCH: usize = 512;
const MAX_QUEUE: usize = 4096;

type SharedQueue = Arc<Mutex<Vec<PushMessage>>>;

pub struct PushHub {
    queue: SharedQueue,
    sink: PushSink,
    #[allow(dead_code)]
    started: AtomicBool,
}

impl PushHub {
    pub fn new(sink: PushSink) -> Self {
        let queue: SharedQueue = Arc::new(Mutex::new(Vec::new()));
        let thread_queue = Arc::clone(&queue);
        let thread_sink = sink.clone();
        std::thread::Builder::new()
            .name("ed-push-flush".into())
            .spawn(move || loop {
                std::thread::sleep(FLUSH_INTERVAL);
                let batch = drain(&thread_queue);
                if !batch.is_empty() {
                    (thread_sink)(&batch);
                }
            })
            .expect("failed to spawn push flush thread");
        Self {
            queue,
            sink,
            started: AtomicBool::new(true),
        }
    }

    pub fn push(&self, topic: &str, target: Option<String>, payload: serde_json::Value) {
        let mut queue = self.queue.lock().unwrap();
        if queue.len() >= MAX_QUEUE {
            // Protect the UI from runaway producers (e.g. pty floods).
            let drop = queue.len() - MAX_BATCH + 1;
            queue.drain(..drop);
            tracing::warn!(dropped = drop, "push queue overflow");
        }
        queue.push(PushMessage {
            topic: topic.to_string(),
            target,
            payload,
        });
    }

    /// Push a message addressed to one plugin iframe.
    pub fn push_to_plugin(&self, plugin_id: &str, kind: &str, payload: serde_json::Value) {
        self.push(
            "plugin-push",
            Some(plugin_id.to_string()),
            serde_json::json!({ "kind": kind, "payload": payload }),
        );
    }

    pub fn flush(&self) {
        let batch = drain(&self.queue);
        if !batch.is_empty() {
            (self.sink)(&batch);
        }
    }
}

fn drain(queue: &SharedQueue) -> Vec<PushMessage> {
    let mut q = queue.lock().unwrap();
    let take = q.len().min(MAX_BATCH);
    let at = q.len() - take;

    q.split_off(at)
}
