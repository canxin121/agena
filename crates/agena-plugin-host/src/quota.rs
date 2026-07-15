//! Per-plugin token-bucket rate limiter and concurrency cap for host
//! callbacks. Each plugin gets a [`PluginQuotaState`] and every host call
//! goes through [`QuotaRegistry::acquire`], which returns a guard that
//! must outlive the callback.
//!
//! Defaults are intentionally permissive (unlimited): operators opt in
//! per-plugin via `plugins.host.quotas.<id>` so high-volume plugins can be
//! throttled without changing their own config.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::sdk::PluginKey;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct QuotaConfig {
    /// Sustained host-call rate. `0` means unlimited.
    #[serde(default)]
    pub rate_per_sec: u32,
    /// Bucket size for short bursts. `0` means use `rate_per_sec`; if both
    /// are zero the limiter is disabled.
    #[serde(default)]
    pub burst: u32,
    /// Maximum concurrent in-flight host calls. `0` means unlimited.
    #[serde(default)]
    pub max_concurrent: u32,
}

impl QuotaConfig {
    pub fn is_unlimited(&self) -> bool {
        self.rate_per_sec == 0 && self.burst == 0 && self.max_concurrent == 0
    }

    pub fn is_unlimited_ref(value: &QuotaConfig) -> bool {
        value.is_unlimited()
    }

    fn effective_burst(&self) -> u32 {
        if self.burst > 0 {
            self.burst
        } else {
            self.rate_per_sec
        }
    }
}

#[derive(Debug)]
struct PluginQuotaState {
    config: QuotaConfig,
    /// Token bucket: how many tokens remain right now.
    tokens: Mutex<f64>,
    /// Last time we refilled.
    last_refill: Mutex<Instant>,
    /// Currently in-flight host calls.
    in_flight: AtomicU32,
}

impl PluginQuotaState {
    fn new(config: QuotaConfig) -> Self {
        let initial = config.effective_burst() as f64;
        Self {
            config,
            tokens: Mutex::new(initial),
            last_refill: Mutex::new(Instant::now()),
            in_flight: AtomicU32::new(0),
        }
    }

    fn try_acquire(&self) -> Result<(), QuotaError> {
        // Concurrency cap.
        if self.config.max_concurrent > 0 {
            let prev = self.in_flight.fetch_add(1, Ordering::SeqCst);
            if prev >= self.config.max_concurrent {
                self.in_flight.fetch_sub(1, Ordering::SeqCst);
                return Err(QuotaError::ConcurrencyExceeded {
                    limit: self.config.max_concurrent,
                });
            }
        }

        // Token bucket.
        if self.config.rate_per_sec > 0 || self.config.burst > 0 {
            let burst = self.config.effective_burst().max(1) as f64;
            let rate = self.config.rate_per_sec as f64;
            let mut tokens = self.tokens.lock().expect("tokens poisoned");
            let mut last = self.last_refill.lock().expect("last_refill poisoned");
            let now = Instant::now();
            let elapsed = now.duration_since(*last).as_secs_f64();
            if rate > 0.0 {
                *tokens = (*tokens + elapsed * rate).min(burst);
            }
            *last = now;
            if *tokens >= 1.0 {
                *tokens -= 1.0;
            } else {
                drop(tokens);
                drop(last);
                if self.config.max_concurrent > 0 {
                    self.in_flight.fetch_sub(1, Ordering::SeqCst);
                }
                return Err(QuotaError::RateExceeded {
                    rate_per_sec: self.config.rate_per_sec,
                    burst: self.config.effective_burst(),
                });
            }
        }
        Ok(())
    }

    fn release(&self) {
        if self.config.max_concurrent > 0 {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Registry of per-plugin quota state, plus a global default applied to
/// any plugin without its own config.
#[derive(Debug, Default)]
pub struct QuotaRegistry {
    default: QuotaConfig,
    plugins: Mutex<HashMap<PluginKey, PluginQuotaState>>,
}

impl QuotaRegistry {
    pub fn new(default: QuotaConfig) -> Self {
        Self {
            default,
            plugins: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_plugin(&self, plugin_id: PluginKey, config: QuotaConfig) {
        let mut plugins = self.plugins.lock().expect("plugins poisoned");
        plugins.insert(plugin_id, PluginQuotaState::new(config));
    }

    pub fn remove_plugin(&self, plugin_id: &PluginKey) {
        let mut plugins = self.plugins.lock().expect("plugins poisoned");
        plugins.remove(plugin_id);
    }

    /// Acquire a quota slot for `plugin_id`. The returned guard releases the
    /// concurrency slot on drop. Plugins without a registered config use the
    /// global default; plugins with `is_unlimited()` skip all checks.
    pub fn acquire<'a>(&'a self, plugin_id: &PluginKey) -> Result<QuotaGuard<'a>, QuotaError> {
        let mut plugins = self.plugins.lock().expect("plugins poisoned");
        if !plugins.contains_key(plugin_id) {
            if self.default.is_unlimited() {
                return Ok(QuotaGuard {
                    registry: self,
                    plugin_id: plugin_id.clone(),
                    armed: false,
                });
            }
            plugins.insert(
                plugin_id.clone(),
                PluginQuotaState::new(self.default.clone()),
            );
        }
        let state = plugins.get(plugin_id).expect("just inserted");
        if state.config.is_unlimited() {
            return Ok(QuotaGuard {
                registry: self,
                plugin_id: plugin_id.clone(),
                armed: false,
            });
        }
        state.try_acquire()?;
        Ok(QuotaGuard {
            registry: self,
            plugin_id: plugin_id.clone(),
            armed: true,
        })
    }

    fn release(&self, plugin_id: &PluginKey) {
        let plugins = self.plugins.lock().expect("plugins poisoned");
        if let Some(state) = plugins.get(plugin_id) {
            state.release();
        }
    }
}

#[derive(Debug)]
pub struct QuotaGuard<'a> {
    registry: &'a QuotaRegistry,
    plugin_id: PluginKey,
    armed: bool,
}

impl<'a> Drop for QuotaGuard<'a> {
    fn drop(&mut self) {
        if self.armed {
            self.registry.release(&self.plugin_id);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QuotaError {
    #[error("plugin host call rate exceeded: rate={rate_per_sec}/s burst={burst}")]
    RateExceeded { rate_per_sec: u32, burst: u32 },
    #[error("plugin host call concurrency exceeded: limit={limit}")]
    ConcurrencyExceeded { limit: u32 },
}
