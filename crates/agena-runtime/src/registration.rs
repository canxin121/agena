//! Generic snapshot-scoped asynchronous registration helpers.

use std::{collections::BTreeMap, future::Future};

/// Runtime-owned projection of a configured agent entry. The concrete
/// registry still owns profile validation/registration, but Runtime owns
/// stable-name normalization and the resolved configuration map traversal.
#[derive(Debug, Clone)]
pub struct RuntimeAgentRegistration {
    pub name: String,
    pub config: crate::AgentConfig,
}

pub fn configured_agent_registrations(
    configured: &BTreeMap<String, crate::AgentConfig>,
) -> Vec<RuntimeAgentRegistration> {
    configured
        .iter()
        .filter_map(|(name, config)| {
            let name = name.trim();
            (!name.is_empty()).then(|| RuntimeAgentRegistration {
                name: name.to_owned(),
                config: config.clone(),
            })
        })
        .collect()
}

/// Spawn a cancellable batch of registrations and retain the guard with the
/// snapshot's runtime service bundle.
pub fn spawn_registration_batch<I, F, Fut>(entries: I, mut register: F) -> crate::AbortOnDrop
where
    I: IntoIterator + Send + 'static,
    I::IntoIter: Send + 'static,
    I::Item: Send + 'static,
    F: FnMut(I::Item) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    crate::spawn_abortable(async move {
        for entry in entries {
            register(entry).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::configured_agent_registrations;
    use crate::AgentConfig;
    use std::collections::BTreeMap;

    #[test]
    fn configured_agent_projection_trims_names_and_skips_blank_keys() {
        let entries = configured_agent_registrations(&BTreeMap::from([
            ("  build  ".to_owned(), AgentConfig::default()),
            ("   ".to_owned(), AgentConfig::default()),
        ]));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "build");
    }
}
