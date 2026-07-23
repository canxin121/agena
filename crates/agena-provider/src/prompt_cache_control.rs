use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCacheControl {
    #[serde(rename = "type")]
    kind: String,
}

impl PromptCacheControl {
    pub fn ephemeral() -> Self {
        Self {
            kind: "ephemeral".to_owned(),
        }
    }
}

pub fn select_cache_target_indices(system_flags: &[bool]) -> Vec<usize> {
    let mut targets = system_flags
        .iter()
        .enumerate()
        .filter_map(|(index, is_system)| is_system.then_some(index))
        .take(2)
        .collect::<Vec<_>>();

    let mut tail = system_flags
        .iter()
        .enumerate()
        .filter_map(|(index, is_system)| (!is_system).then_some(index))
        .collect::<Vec<_>>();
    let keep = tail.len().saturating_sub(2);
    tail.drain(..keep);

    for index in tail {
        if !targets.contains(&index) {
            targets.push(index);
        }
    }

    targets
}
