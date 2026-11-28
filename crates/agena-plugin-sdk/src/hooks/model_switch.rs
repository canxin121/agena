use serde::{Deserialize, Serialize};

pub use agena_domain::ModelSwitchSource;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedModelSwitchInput {
    pub session_id: i64,
    pub from_model: String,
    pub to_model: String,
    pub canonical_model: Option<String>,
    pub requested_model: Option<String>,
    pub source: ModelSwitchSource,
    pub context_tokens: u64,
    pub prompt_cache_warm: bool,
    pub cache_ttl: ModelSwitchCacheTtl,
    pub estimated_cache_write_usd: f64,
    pub pricing: ModelSwitchPricing,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub enum ModelSwitchCacheTtl {
    #[default]
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "1h")]
    OneHour,
}

impl ModelSwitchCacheTtl {
    pub fn duration(self) -> std::time::Duration {
        std::time::Duration::from_secs(match self {
            Self::FiveMinutes => 300,
            Self::OneHour => 3600,
        })
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSwitchPricing {
    Configured,
    Catalog,
    #[default]
    Default,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", content = "reason", rename_all = "snake_case")]
pub enum ImportedModelSwitchDecision {
    Allow,
    Ask(String),
    Deny(String),
}

impl ImportedModelSwitchDecision {
    pub fn priority(&self) -> u8 {
        match self {
            Self::Allow => 0,
            Self::Ask(_) => 1,
            Self::Deny(_) => 2,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportedModelSwitchPatch {
    pub decision: Option<ImportedModelSwitchDecision>,
    pub context: Vec<String>,
    pub system_messages: Vec<String>,
}

impl ImportedModelSwitchPatch {
    pub fn apply_decision(&mut self, decision: ImportedModelSwitchDecision) {
        if self
            .decision
            .as_ref()
            .is_none_or(|old| decision.priority() > old.priority())
        {
            self.decision = Some(decision);
        }
    }
}
