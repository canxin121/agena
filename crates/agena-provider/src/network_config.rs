//! Provider request transport policy shared by configuration and adapters.

use serde::{Deserialize, Serialize};

/// Default provider request timeout in seconds.
pub const DEFAULT_PROVIDER_REQUEST_TIMEOUT_SECS: u64 = 120;
/// Default provider connect timeout in seconds.
pub const DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS: u64 = 15;

/// Network timeouts belong to the provider they affect. Keeping them on the
/// provider avoids a single global runtime knob changing unrelated backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// Network configuration for provider requests.
pub struct ProviderNetworkConfig {
    pub request_timeout_secs: u64,
    pub connect_timeout_secs: u64,
}

impl Default for ProviderNetworkConfig {
    fn default() -> Self {
        Self {
            request_timeout_secs: DEFAULT_PROVIDER_REQUEST_TIMEOUT_SECS,
            connect_timeout_secs: DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS, DEFAULT_PROVIDER_REQUEST_TIMEOUT_SECS,
        ProviderNetworkConfig,
    };

    #[test]
    fn default_network_timeouts_are_stable_provider_contracts() {
        assert_eq!(
            ProviderNetworkConfig::default().request_timeout_secs,
            DEFAULT_PROVIDER_REQUEST_TIMEOUT_SECS
        );
        assert_eq!(
            ProviderNetworkConfig::default().connect_timeout_secs,
            DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS
        );
    }
}
