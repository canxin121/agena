//! Plugin log capture and storage.

use std::collections::{HashMap, VecDeque};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::sdk::PluginKey;
use crate::status::now_ms;

const DEFAULT_MAX_ENTRIES_PER_PLUGIN: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A log record produced by a plugin.
pub struct PluginLogRecord {
    pub seq: u64,
    pub timestamp_ms: i64,
    pub plugin_id: PluginKey,
    pub level: String,
    pub source: String,
    pub message: String,
    pub fields: serde_json::Value,
}

#[derive(Debug)]
/// In-memory store of plugin log records.
pub struct PluginLogStore {
    max_entries_per_plugin: usize,
    next_seq: AtomicU64,
    inner: RwLock<HashMap<PluginKey, VecDeque<PluginLogRecord>>>,
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
        plugin_id: &PluginKey,
        level: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
        fields: serde_json::Value,
    ) -> PluginLogRecord {
        let record = PluginLogRecord {
            seq: self.next_seq.fetch_add(1, Ordering::SeqCst),
            timestamp_ms: now_ms(),
            plugin_id: plugin_id.clone(),
            level: level.into(),
            source: source.into(),
            message: message.into(),
            fields,
        };
        if let Ok(mut guard) = self.inner.write() {
            let bucket = guard.entry(plugin_id.clone()).or_default();
            bucket.push_back(record.clone());
            while bucket.len() > self.max_entries_per_plugin {
                bucket.pop_front();
            }
        }
        record
    }

    pub fn list(
        &self,
        plugin_id: &PluginKey,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<PluginLogRecord> {
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
                    .filter(|record| after_seq.is_none_or(|seq| record.seq > seq))
                    .take(limit)
                    .collect()
            })
            .unwrap_or_default()
    }
}
