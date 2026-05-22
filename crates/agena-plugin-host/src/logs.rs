use std::collections::{HashMap, VecDeque};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::status::now_ms;

const DEFAULT_MAX_ENTRIES_PER_PLUGIN: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginLogEntry {
    pub seq: u64,
    pub timestamp_ms: i64,
    pub plugin_id: String,
    pub level: String,
    pub source: String,
    pub message: String,
    pub fields: serde_json::Value,
}

#[derive(Debug)]
pub struct PluginLogStore {
    max_entries_per_plugin: usize,
    next_seq: AtomicU64,
    inner: RwLock<HashMap<String, VecDeque<PluginLogEntry>>>,
}

impl Default for PluginLogStore {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_ENTRIES_PER_PLUGIN)
    }
}

impl PluginLogStore {
    pub fn new(max_entries_per_plugin: usize) -> Self {
        Self {
            max_entries_per_plugin: max_entries_per_plugin.max(1),
            next_seq: AtomicU64::new(1),
            inner: RwLock::new(HashMap::new()),
        }
    }

    pub fn append(
        &self,
        plugin_id: impl Into<String>,
        level: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
        fields: serde_json::Value,
    ) -> PluginLogEntry {
        let plugin_id = plugin_id.into();
        let entry = PluginLogEntry {
            seq: self.next_seq.fetch_add(1, Ordering::SeqCst),
            timestamp_ms: now_ms(),
            plugin_id: plugin_id.clone(),
            level: level.into(),
            source: source.into(),
            message: message.into(),
            fields,
        };
        if let Ok(mut guard) = self.inner.write() {
            let bucket = guard.entry(plugin_id).or_default();
            bucket.push_back(entry.clone());
            while bucket.len() > self.max_entries_per_plugin {
                bucket.pop_front();
            }
        }
        entry
    }

    pub fn list(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<PluginLogEntry> {
        let limit = if limit == 0 {
            self.max_entries_per_plugin
        } else {
            limit.min(self.max_entries_per_plugin)
        };
        self.inner
            .read()
            .ok()
            .and_then(|guard| guard.get(plugin_id).cloned())
            .map(|entries| {
                entries
                    .into_iter()
                    .filter(|entry| after_seq.is_none_or(|seq| entry.seq > seq))
                    .take(limit)
                    .collect()
            })
            .unwrap_or_default()
    }
}
