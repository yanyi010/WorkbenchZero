//! Notification service (spec §72): kernel-routed notifications with
//! optional actions, surfaced as in-app toasts by the shell. History is
//! session-scoped.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::Kernel;

#[derive(Debug, Clone, Serialize)]
pub struct NotificationAction {
    pub id: String,
    pub title: String,
    /// Command to invoke when the action is clicked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NotificationRecord {
    pub id: String,
    pub title: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub actions: Vec<NotificationAction>,
    pub timestamp: DateTime<Utc>,
}

const HISTORY_CAP: usize = 100;

/// Record + push a notification to the shell.
pub fn push_notification(
    kernel: &Kernel,
    title: &str,
    body: &str,
    source: Option<String>,
    actions: Vec<Value>,
) {
    let actions: Vec<NotificationAction> = actions
        .iter()
        .filter_map(|a| {
            Some(NotificationAction {
                id: a.get("id")?.as_str()?.to_string(),
                title: a.get("title")?.as_str()?.to_string(),
                command: a.get("command").and_then(|c| c.as_str()).map(|s| s.to_string()),
                args: a.get("args").cloned(),
            })
        })
        .collect();
    let record = NotificationRecord {
        id: format!("notif-{}", uuid::Uuid::new_v4().simple()),
        title: title.to_string(),
        body: body.to_string(),
        source: source.clone(),
        actions,
        timestamp: Utc::now(),
    };
    {
        let mut history = kernel.notifications.lock().unwrap();
        history.push(record.clone());
        let len = history.len();
        if len > HISTORY_CAP {
            history.drain(..len - HISTORY_CAP);
        }
    }
    kernel.push.push("notification", None, serde_json::to_value(&record).unwrap_or(Value::Null));
}
