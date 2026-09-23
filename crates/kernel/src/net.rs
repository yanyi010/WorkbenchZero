//! Network capability with host-scoped permissions and SSE streaming.
//! Plugins never fetch directly from their iframe: the CSP forbids
//! `connect-src`, and all traffic goes through the kernel where the network
//! allow-list is enforced. Request headers are never logged (secret hygiene).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;

use crate::{CallerCtx, Kernel, KernelError, KResult};

pub struct NetService {
    client: reqwest::blocking::Client,
    streams: Mutex<HashMap<String, Arc<StreamHandle>>>,
    next: AtomicU64,
}

struct StreamHandle {
    abort: tokio::sync::watch::Sender<bool>,
}

impl NetService {
    pub fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .build()
            .expect("failed to build http client");
        Self { client, streams: Mutex::new(HashMap::new()), next: AtomicU64::new(1) }
    }

    pub fn abort_all(&self) {
        let mut streams = self.streams.lock().unwrap();
        for (_, handle) in streams.drain() {
            let _ = handle.abort.send(true);
        }
    }

    pub fn abort(&self, stream_id: &str) -> bool {
        let mut streams = self.streams.lock().unwrap();
        match streams.remove(stream_id) {
            Some(handle) => {
                let _ = handle.abort.send(true);
                true
            }
            None => false,
        }
    }
}

fn extract_host(url: &str) -> Result<String, KernelError> {
    reqwest::Url::parse(url)
        .map_err(|e| KernelError::Message(format!("invalid url `{url}`: {e}")))
        .and_then(|u| {
            u.host_str()
                .map(|h| h.to_string())
                .ok_or_else(|| KernelError::Message(format!("url `{url}` has no host")))
        })
}

fn check_permission(kernel: &Kernel, caller: &CallerCtx, url: &str) -> KResult<()> {
    let Some(plugin_id) = &caller.plugin_id else {
        return Ok(());
    };
    let host = extract_host(url)?;
    kernel
        .permissions
        .check_network(plugin_id, &host)
        .map_err(|e| KernelError::Permission(e.to_string()))
}

fn build_request(
    client: &reqwest::blocking::Client,
    method: &str,
    url: &str,
    headers: &Value,
    body: Option<&str>,
) -> KResult<reqwest::blocking::RequestBuilder> {
    let method = reqwest::Method::from_bytes(method.to_uppercase().as_bytes())
        .map_err(|_| KernelError::Message(format!("unsupported http method `{method}`")))?;
    let mut builder = client.request(method, url);
    if let Some(map) = headers.as_object() {
        for (k, v) in map {
            let value = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            builder = builder.header(k, value);
        }
    }
    if let Some(body) = body {
        builder = builder.body(body.to_string());
    }
    Ok(builder)
}

/// Blocking fetch (small JSON APIs, model list probes...).
pub fn fetch(
    kernel: &Kernel,
    caller: &CallerCtx,
    url: &str,
    method: &str,
    headers: &Value,
    body: Option<&str>,
    timeout_ms: Option<u64>,
) -> KResult<Value> {
    check_permission(kernel, caller, url)?;
    let client = if let Some(ms) = timeout_ms {
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_millis(ms))
            .build()
            .map_err(|e| KernelError::Message(format!("client build failed: {e}")))?
    } else {
        kernel.net.client.clone()
    };
    let builder = build_request(&client, method, url, headers, body)?;
    let response = builder.send().map_err(|e| KernelError::Message(format!("request to `{url}` failed: {e}")))?;
    let status = response.status().as_u16();
    let mut response_headers = serde_json::Map::new();
    for (name, value) in response.headers() {
        response_headers.insert(
            name.as_str().to_string(),
            Value::String(value.to_str().unwrap_or_default().to_string()),
        );
    }
    let text = response.text().map_err(|e| KernelError::Message(format!("read body failed: {e}")))?;
    Ok(serde_json::json!({
        "status": status,
        "headers": response_headers,
        "body": text,
    }))
}

/// Start a streaming fetch with server-sent-events parsing. Chunks are
/// pushed to the calling plugin as `net-chunk` / `net-end` / `net-error`.
pub fn fetch_stream(
    kernel: &Arc<Kernel>,
    caller: &CallerCtx,
    url: &str,
    method: &str,
    headers: &Value,
    body: Option<&str>,
) -> KResult<Value> {
    check_permission(kernel, caller, url)?;
    let target_plugin = caller.plugin_id.clone();
    let stream_id = format!("net-{}", kernel.net.next.fetch_add(1, Ordering::SeqCst));
    let (abort_tx, mut abort_rx) = tokio::sync::watch::channel(false);
    kernel
        .net
        .streams
        .lock()
        .unwrap()
        .insert(stream_id.clone(), Arc::new(StreamHandle { abort: abort_tx }));

    let url_owned = url.to_string();
    let method_owned = method.to_string();
    let headers_owned = headers.clone();
    let body_owned = body.map(|b| b.to_string());
    let sid = stream_id.clone();
    let kernel_task = kernel.clone();

    kernel.runtime.spawn(async move {
        let kernel = kernel_task;
        let result = run_stream(
            &kernel,
            &sid,
            target_plugin.as_deref(),
            &url_owned,
            &method_owned,
            &headers_owned,
            body_owned.as_deref(),
            &mut abort_rx,
        )
        .await;
        let _ = result;
        kernel.net.streams.lock().unwrap().remove(&sid);
    });

    Ok(serde_json::json!({ "streamId": stream_id }))
}

async fn run_stream(
    kernel: &Arc<Kernel>,
    stream_id: &str,
    target_plugin: Option<&str>,
    url: &str,
    method: &str,
    headers: &Value,
    body: Option<&str>,
    abort_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> KResult<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| KernelError::Message(format!("client build failed: {e}")))?;
    let method = reqwest::Method::from_bytes(method.to_uppercase().as_bytes())
        .map_err(|_| KernelError::Message(format!("unsupported http method `{method}`")))?;
    let mut builder = client.request(method, url);
    if let Some(map) = headers.as_object() {
        for (k, v) in map {
            let value = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            builder = builder.header(k, value);
        }
    }
    if let Some(body) = body {
        builder = builder.body(body.to_string());
    }
    let response = builder
        .send()
        .await
        .map_err(|e| KernelError::Message(format!("request to `{url}` failed: {e}")))?;
    let status = response.status().as_u16();
    let mut stream = response.bytes_stream();

    let push = |kind: &str, payload: Value| {
        let msg = serde_json::json!({ "kind": kind, "payload": payload });
        match target_plugin {
            Some(plugin) => {
                // Deliver only to the owning plugin's iframe.
                kernel.push.push_to_plugin(plugin, "net", msg);
            }
            None => kernel.push.push("net", None, msg),
        }
    };

    if status >= 400 {
        let body = stream
            .next()
            .await
            .and_then(|c| c.ok())
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
        push(
            "net-error",
            serde_json::json!({ "streamId": stream_id, "status": status, "message": body }),
        );
        return Ok(());
    }

    // SSE state machine: parse `event:`/`data:` lines, dispatch on blank line.
    let mut buffer: Vec<u8> = Vec::new();
    let mut event_name = String::new();
    let mut data_lines: Vec<String> = Vec::new();
    let send_event = |event: &str, data: &str, push: &dyn Fn(&str, Value)| {
        push(
            "net-chunk",
            serde_json::json!({ "streamId": stream_id, "event": event, "data": data }),
        );
    };
    push("net-start", serde_json::json!({ "streamId": stream_id, "status": status }));

    loop {
        tokio::select! {
            _ = abort_rx.changed() => {
                if *abort_rx.borrow() {
                    push("net-abort", serde_json::json!({ "streamId": stream_id }));
                    return Ok(());
                }
            }
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        buffer.extend_from_slice(&bytes);
                        while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                            let line: Vec<u8> = buffer.drain(..=pos).collect();
                            let line = String::from_utf8_lossy(&line[..line.len() - 1]);
                            let line = line.trim_end_matches('\r');
                            if line.is_empty() {
                                if !data_lines.is_empty() {
                                    let data = data_lines.join("\n");
                                    send_event(&event_name, &data, &push);
                                    data_lines.clear();
                                }
                                event_name.clear();
                            } else if let Some(rest) = line.strip_prefix("event:") {
                                event_name = rest.trim().to_string();
                            } else if let Some(rest) = line.strip_prefix("data:") {
                                data_lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
                            }
                        }
                    }
                    Some(Err(e)) => {
                        push("net-error", serde_json::json!({ "streamId": stream_id, "message": e.to_string() }));
                        return Ok(());
                    }
                    None => {
                        // EOF: flush trailing event if any.
                        if !data_lines.is_empty() {
                            let data = data_lines.join("\n");
                            send_event(&event_name, &data, &push);
                        }
                        push("net-end", serde_json::json!({ "streamId": stream_id }));
                        return Ok(());
                    }
                }
            }
        }
    }
}
