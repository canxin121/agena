//! Typed configuration value structs.

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

mod provider;
mod resolved;
mod runtime;

pub use self::provider::*;
pub use self::resolved::*;
pub use self::runtime::*;
