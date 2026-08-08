use std::{borrow::Borrow, fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{field} cannot be empty")]
/// Error for an invalid identifier.
pub struct IdentifierError {
    field: &'static str,
}

impl IdentifierError {
    const fn new(field: &'static str) -> Self {
        Self { field }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Error parsing a `provider/model` model reference.
pub enum ModelRefParseError {
    #[error("model reference must be in `provider/model` format")]
    MissingSeparator,
    #[error(transparent)]
    InvalidProviderId(#[from] IdentifierError),
    #[error("{0}")]
    InvalidModelId(String),
}

fn normalize_non_empty(
    value: impl Into<String>,
    field: &'static str,
) -> Result<String, IdentifierError> {
    let value = value.into();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(IdentifierError::new(field));
    }
    Ok(trimmed.to_owned())
}

macro_rules! define_string_identifier {
    ($name:ident, $field:literal, $expect_message:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self::try_new(value).expect($expect_message)
            }

            pub fn try_new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                Ok(Self(normalize_non_empty(value, $field)?))
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                self.0.as_str()
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.0.as_str())
            }
        }

        impl FromStr for $name {
            type Err = IdentifierError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::try_new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentifierError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::try_new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

define_string_identifier!(ProviderId, "provider id", "provider id cannot be empty");
define_string_identifier!(AdapterId, "adapter id", "adapter id cannot be empty");
define_string_identifier!(ModelId, "model id", "model id cannot be empty");

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// A `provider[/adapter]/model` reference identifying a model.
pub struct ModelRef {
    pub provider_id: ProviderId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<AdapterId>,
    pub model_id: ModelId,
}

impl ModelRef {
    pub fn new(provider_id: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self::try_new(provider_id, model_id).expect("model reference must be valid")
    }

    pub fn try_new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, IdentifierError> {
        Ok(Self {
            provider_id: ProviderId::try_new(provider_id)?,
            adapter_id: None,
            model_id: ModelId::try_new(model_id)?,
        })
    }

    pub fn new_with_adapter(
        provider_id: impl Into<String>,
        adapter_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Self {
        Self::try_new_with_adapter(provider_id, adapter_id, model_id)
            .expect("model reference must be valid")
    }

    pub fn try_new_with_adapter(
        provider_id: impl Into<String>,
        adapter_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, IdentifierError> {
        Ok(Self {
            provider_id: ProviderId::try_new(provider_id)?,
            adapter_id: Some(AdapterId::try_new(adapter_id)?),
            model_id: ModelId::try_new(model_id)?,
        })
    }
}

impl fmt::Display for ModelRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(adapter_id) = &self.adapter_id {
            write!(
                f,
                "provider={} adapter={} model={}",
                self.provider_id, adapter_id, self.model_id
            )
        } else {
            write!(f, "{}/{}", self.provider_id, self.model_id)
        }
    }
}

impl FromStr for ModelRef {
    type Err = ModelRefParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((provider_id, model_id)) = value.split_once('/') else {
            return Err(ModelRefParseError::MissingSeparator);
        };
        let provider_id = ProviderId::try_new(provider_id)?;
        let model_id = ModelId::try_new(model_id)
            .map_err(|err| ModelRefParseError::InvalidModelId(err.to_string()))?;
        Ok(Self {
            provider_id,
            adapter_id: None,
            model_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{ModelRef, ProviderId};

    #[test]
    fn identifiers_trim_and_reject_blank_values() {
        assert_eq!(ProviderId::new(" provider ").as_ref(), "provider");
        assert!(ProviderId::try_new(" \t ").is_err());
    }

    #[test]
    fn model_ref_parse_and_wire_shape_are_stable() {
        let model = ModelRef::from_str("provider/model").expect("valid reference");
        assert_eq!(model.provider_id.as_ref(), "provider");
        assert_eq!(model.model_id.as_ref(), "model");
        assert!(model.adapter_id.is_none());
        assert_eq!(
            serde_json::to_value(model).expect("serialize model reference"),
            serde_json::json!({ "provider_id": "provider", "model_id": "model" })
        );
    }
}
