//! Structured diagnostics (spec §82): startup phases, RPC latency, plugin
//! activation time, search latency. Local-only; no telemetry ever leaves the
//! machine.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use wz_common::MutexRecover;

const RING_CAP: usize = 256;

pub struct StartupTracker {
    start: Instant,
    phases: Vec<(String, f64)>,
}

impl Default for StartupTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl StartupTracker {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            phases: Vec::new(),
        }
    }

    pub fn mark(&mut self, name: &str) {
        let t = self.start.elapsed().as_secs_f64() * 1000.0;
        tracing::debug!(phase = name, ms = t, "startup phase");
        self.phases.push((name.to_string(), t));
    }

    pub fn finish(self) -> (Vec<(String, f64)>, f64) {
        (self.phases, self.start.elapsed().as_secs_f64() * 1000.0)
    }
}

#[derive(Default)]
struct LatencyRing {
    samples: Vec<f64>,
    total: u64,
}

impl LatencyRing {
    fn record(&mut self, ms: f64) {
        if self.samples.len() >= RING_CAP {
            self.samples.remove(0);
        }
        self.samples.push(ms);
        self.total += 1;
    }

    fn stats(&self) -> serde_json::Value {
        if self.samples.is_empty() {
            return serde_json::json!({ "count": 0, "total": self.total });
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let pct = |p: usize| sorted[p.min(sorted.len() - 1)];
        serde_json::json!({
            "count": sorted.len(),
            "total": self.total,
            "p50": pct(sorted.len() * 50 / 100),
            "p95": pct(sorted.len() * 95 / 100),
            "max": sorted[sorted.len() - 1],
        })
    }
}

pub struct Diagnostics {
    app_version: String,
    boot_time: chrono::DateTime<chrono::Utc>,
    startup: Mutex<Vec<(String, f64)>>,
    startup_total_ms: Mutex<f64>,
    rpc: Mutex<HashMap<String, LatencyRing>>,
    activation: Mutex<HashMap<String, f64>>,
    search: Mutex<LatencyRing>,
}

impl Diagnostics {
    pub fn new(app_version: String) -> Self {
        Self {
            app_version,
            boot_time: chrono::Utc::now(),
            startup: Mutex::new(Vec::new()),
            startup_total_ms: Mutex::new(0.0),
            rpc: Mutex::new(HashMap::new()),
            activation: Mutex::new(HashMap::new()),
            search: Mutex::new(LatencyRing::default()),
        }
    }

    pub fn record_startup(&self, phases: Vec<(String, f64)>, total_ms: f64) {
        *self.startup.lock_or_recover() = phases;
        *self.startup_total_ms.lock_or_recover() = total_ms;
    }

    pub fn record_rpc(&self, method: &str, ms: f64) {
        self.rpc
            .lock()
            .unwrap()
            .entry(method.to_string())
            .or_default()
            .record(ms);
    }

    pub fn record_activation(&self, plugin_id: &str, ms: f64) {
        self.activation
            .lock()
            .unwrap()
            .insert(plugin_id.to_string(), ms);
    }

    pub fn record_search(&self, ms: f64) {
        self.search.lock_or_recover().record(ms);
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let rpc: serde_json::Map<String, serde_json::Value> = self
            .rpc
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.stats()))
            .collect();
        let activation: serde_json::Map<String, serde_json::Value> = self
            .activation
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
            .collect();
        let startup: Vec<serde_json::Value> = self
            .startup
            .lock()
            .unwrap()
            .iter()
            .map(|(name, ms)| serde_json::json!({ "phase": name, "ms": ms }))
            .collect();
        serde_json::json!({
            "appVersion": self.app_version,
            "bootTime": self.boot_time.to_rfc3339(),
            "uptimeSec": (chrono::Utc::now() - self.boot_time).num_seconds(),
            "startupPhases": startup,
            "startupTotalMs": *self.startup_total_ms.lock_or_recover(),
            "rpcLatency": rpc,
            "pluginActivationMs": activation,
            "searchLatency": self.search.lock_or_recover().stats(),
        })
    }
}
