use std::{collections::BTreeMap, path::PathBuf, str::FromStr};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

use crate::execution_prefs::ExecutionSelection;
use crate::provider::{
    CapabilityFamily, ConfiguredModelDefinition, GeminiStreamMode, OpenAiResponsesBackend,
    auth::{AuthData, CredentialIssuer},
};

use super::ConfigError;

mod provider;
mod provider_native_tools;
mod resolved;
mod runtime;

pub use self::provider::*;
pub use self::provider_native_tools::*;
pub use self::resolved::*;
pub use self::runtime::*;
