use std::{collections::BTreeMap, path::PathBuf, str::FromStr, time::Duration};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

use crate::execution_prefs::ExecutionSelection;
use crate::provider::{
    CapabilityFamily, ConfiguredModelDefinition, GeminiStreamMode, OpenAiResponsesBackend,
    ProviderHttpClientConfig, ProviderRequestRetryConfig, ProviderRuntimeConfig,
    ProviderStreamReplayConfig,
    auth::{AuthData, CredentialIssuer},
};

use super::ConfigError;

mod native_tools;
mod provider;
mod resolved;
mod runtime;

pub use self::native_tools::*;
pub use self::provider::*;
pub use self::resolved::*;
pub use self::runtime::*;
