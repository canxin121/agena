//! Stable doom-loop detection policy and result values.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DoomLoopPolicy {
    /// Number of immediately-consecutive identical tool calls that constitute
    /// a doom loop. Values below 2 disable the check.
    pub repeat_threshold: u8,
}

impl Default for DoomLoopPolicy {
    fn default() -> Self {
        Self {
            repeat_threshold: 3,
        }
    }
}

impl DoomLoopPolicy {
    pub const fn disabled() -> Self {
        Self {
            repeat_threshold: 0,
        }
    }

    pub const fn is_enabled(&self) -> bool {
        self.repeat_threshold >= 2
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoomLoopHit {
    pub tool_label: String,
    pub repeat_count: u8,
}

impl DoomLoopHit {
    pub fn message(&self) -> String {
        format!(
            "doom-loop detected: tool `{}` was invoked with the same input {} times in a row; aborting run",
            self.tool_label, self.repeat_count
        )
    }
}
