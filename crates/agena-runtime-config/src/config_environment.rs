//! Environment access used by configuration/bootstrap adapters.

/// Dependency-light environment boundary for configuration resolution.
pub trait ConfigEnvironment: Send + Sync {
    fn var(&self, key: &str) -> Option<String>;
    fn vars(&self) -> Vec<(String, String)>;
}

/// Process environment implementation for normal application startup.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessEnvironment;

impl ConfigEnvironment for ProcessEnvironment {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn vars(&self) -> Vec<(String, String)> {
        std::env::vars().collect()
    }
}
