use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PromptCacheControl {
    #[serde(rename = "type")]
    kind: String,
}

impl PromptCacheControl {
    pub(crate) fn ephemeral() -> Self {
        Self {
            kind: "ephemeral".to_owned(),
        }
    }
}

pub(crate) fn select_cache_target_indices(system_flags: &[bool]) -> Vec<usize> {
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

#[cfg(test)]
mod tests {
    use super::select_cache_target_indices;

    #[test]
    fn select_cache_targets_prefers_first_system_and_last_non_system_messages() {
        let targets = select_cache_target_indices(&[true, false, true, false, false, false]);
        assert_eq!(targets, vec![0, 2, 4, 5]);
    }

    #[test]
    fn select_cache_targets_deduplicates_when_only_system_messages_exist() {
        let targets = select_cache_target_indices(&[true, true]);
        assert_eq!(targets, vec![0, 1]);
    }
}
