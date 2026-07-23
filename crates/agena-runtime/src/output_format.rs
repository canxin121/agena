//! Stable presentation-format selection used by process-facing commands.
//!
//! This is deliberately independent of the legacy configuration schema: a
//! command-line output choice is a presentation contract, not a resolved
//! configuration value.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Json,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("unknown output format `{0}`")]
pub struct OutputFormatParseError(String);

impl FromStr for OutputFormat {
    type Err = OutputFormatParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "json" => Ok(Self::Json),
            _ => Err(OutputFormatParseError(value.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OutputFormat;

    #[test]
    fn parses_the_stable_json_format() {
        assert_eq!("json".parse(), Ok(OutputFormat::Json));
        assert!("yaml".parse::<OutputFormat>().is_err());
    }
}
