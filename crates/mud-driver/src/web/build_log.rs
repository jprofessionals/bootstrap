use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildLogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub event: String,
    pub message: String,
}

pub struct BuildLog {
    capacity: usize,
    entries: Mutex<HashMap<String, VecDeque<BuildLogEntry>>>,
}

impl BuildLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn append(&self, area_key: &str, level: LogLevel, event: &str, message: &str) {
        let entry = BuildLogEntry {
            timestamp: Utc::now(),
            level,
            event: event.to_string(),
            message: message.to_string(),
        };

        let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let deque = map.entry(area_key.to_string()).or_insert_with(VecDeque::new);

        if deque.len() >= self.capacity {
            deque.pop_front();
        }
        deque.push_back(entry);
    }

    pub fn recent(&self, area_key: &str, limit: usize, level: Option<LogLevel>) -> Vec<BuildLogEntry> {
        let map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let Some(deque) = map.get(area_key) else {
            return Vec::new();
        };

        deque
            .iter()
            .filter(|e| level.map_or(true, |l| e.level == l))
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}
