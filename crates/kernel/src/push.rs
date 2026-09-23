//! Push hub: batches kernel→UI messages and flushes them through the
//! installed sink (the Tauri shell evals them into the main frame).
//! Plugin-addressed messages are only ever delivered to the main frame,
//! which routes them to the correct isolated iframe — plugin iframes have no
//! direct channel to the kernel.
//!
//! Wire shape is the canonical `PushMessage {seq, topic, plugin?, data}`
//! (mirrored in `@workbench-zero/protocol`); every message carries a
//! monotonically increasing `seq` so the UI can detect dropped batches.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wz_common::MutexRecover;

use crate::{PushMessage, PushSink};

const FLUSH_INTERVAL: Duration = Duration::from_millis(12);
const MAX_BATCH: usize = 512;
const MAX_QUEUE: usize = 4096;

type SharedQueue = Arc<Mutex<Vec<PushMessage>>>;

pub struct PushHub {
    queue: SharedQueue,
    sink: PushSink,
    seq: Arc<AtomicU64>,
    /// False only if the flush thread could not be spawned (extreme resource
    /// exhaustion): pushes still enqueue and `flush()` still works.
    flusher_running: AtomicBool,
}

impl PushHub {
    pub fn new(sink: PushSink) -> Self {
        let queue: SharedQueue = Arc::new(Mutex::new(Vec::new()));
        let seq = Arc::new(AtomicU64::new(1));
        let running = {
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
                .map(|_| true)
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "push flush thread failed to spawn; remote UI updates will only flush on shutdown");
                    false
                })
        };
        Self {
            queue,
            sink,
            seq,
            flusher_running: AtomicBool::new(running),
        }
    }

    /// Enqueue a message. Never panics, never blocks producers (overflow
    /// drops the oldest messages with a loud log, protecting the UI from
    /// runaway producers like pty floods, at the cost of visible seq gaps).
    pub fn push(&self, topic: &str, plugin: Option<String>, data: serde_json::Value) {
        let msg = PushMessage {
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            topic: topic.to_string(),
            plugin,
            data,
        };
        let mut queue = self.queue.lock_or_recover();
        if queue.len() >= MAX_QUEUE {
            let drop = queue.len() - MAX_BATCH + 1;
            queue.drain(..drop);
            tracing::warn!(dropped = drop, topic, "push queue overflow; oldest messages dropped");
        }
        queue.push(msg);
    }

    /// Push a message addressed to one plugin iframe (routed by the shell).
    pub fn push_to_plugin(&self, plugin_id: &str, topic: &str, data: serde_json::Value) {
        self.push(topic, Some(plugin_id.to_string()), data);
    }

    pub fn flush(&self) {
        let batch = drain(&self.queue);
        if !batch.is_empty() {
            (self.sink)(&batch);
        }
    }
}

fn drain(queue: &SharedQueue) -> Vec<PushMessage> {
    let mut q = queue.lock_or_recover();
    let take = q.len().min(MAX_BATCH);
    let at = q.len() - take;

    q.split_off(at)
}
