//! Stable doom-loop detection policy and result values.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// Policy detecting repeated identical tool calls (doom loop).
pub struct DoomLoopPolicy {
    /// Number of identical calls with identical results that constitute a
    /// doom loop. Help lookups may be interleaved. Values below 2 disable it.
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
/// A detected doom-loop hit (repeated tool invocation).
pub struct DoomLoopHit {
    pub tool_label: String,
    pub repeat_count: u8,
}

impl DoomLoopHit {
    pub fn message(&self) -> String {
        format!(
            "doom-loop detected: tool `{}` returned the same result for the same input {} times; aborting run",
            self.tool_label, self.repeat_count
        )
    }
}
