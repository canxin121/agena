use serde::{Deserialize, Serialize};

/// Structured payload shared by dynamic tool inputs and outputs.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct StructuredObject {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<StructuredField>,
}

impl StructuredObject {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&StructuredValue> {
        self.fields
            .iter()
            .find_map(|field| (field.name == name).then_some(&field.value))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A named structured value field.
pub struct StructuredField {
    pub name: String,
    pub value: StructuredValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// A structured value (null, boolean, number, string, array, object).
pub enum StructuredValue {
    Null,
    Boolean {
        value: bool,
    },
    Integer {
        value: i64,
    },
    /// Decimal/scientific notation text to avoid precision loss.
    Number {
        value: String,
    },
    Text {
        value: String,
    },
    Array {
        items: Vec<StructuredValue>,
    },
    Object {
        fields: Vec<StructuredField>,
    },
}

impl StructuredValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { value } => Some(value.as_str()),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[StructuredValue]> {
        match self {
            Self::Array { items } => Some(items.as_slice()),
            _ => None,
        }
    }
}

impl TryFrom<serde_json::Value> for StructuredObject {
    type Error = String;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::Object(fields) => Ok(Self {
                fields: fields
                    .into_iter()
                    .map(|(name, value)| {
                        Ok(StructuredField {
                            name,
                            value: StructuredValue::try_from(value)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            }),
            other => Err(format!(
                "structured object must deserialize from a JSON object, got {other}"
            )),
        }
    }
}

impl TryFrom<serde_json::Value> for StructuredValue {
    type Error = String;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        Ok(match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(value) => Self::Boolean { value },
            serde_json::Value::Number(value) => {
                if let Some(integer) = value.as_i64() {
                    Self::Integer { value: integer }
                } else {
                    Self::Number {
                        value: value.to_string(),
                    }
                }
            }
            serde_json::Value::String(value) => Self::Text { value },
            serde_json::Value::Array(items) => Self::Array {
                items: items
                    .into_iter()
                    .map(Self::try_from)
                    .collect::<Result<Vec<_>, String>>()?,
            },
            serde_json::Value::Object(fields) => Self::Object {
                fields: fields
                    .into_iter()
                    .map(|(name, value)| {
                        Ok(StructuredField {
                            name,
                            value: StructuredValue::try_from(value)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            },
        })
    }
}

impl From<StructuredObject> for serde_json::Value {
    fn from(value: StructuredObject) -> Self {
        serde_json::Value::Object(
            value
                .fields
                .into_iter()
                .map(|field| (field.name, serde_json::Value::from(field.value)))
                .collect(),
        )
    }
}

impl From<StructuredValue> for serde_json::Value {
    fn from(value: StructuredValue) -> Self {
        match value {
            StructuredValue::Null => serde_json::Value::Null,
            StructuredValue::Boolean { value } => serde_json::Value::Bool(value),
            StructuredValue::Integer { value } => serde_json::json!(value),
            StructuredValue::Number { value } => serde_json::from_str::<serde_json::Value>(&value)
                .ok()
                .and_then(|parsed| match parsed {
                    serde_json::Value::Number(number) => Some(serde_json::Value::Number(number)),
                    _ => None,
                })
                .unwrap_or(serde_json::Value::String(value)),
            StructuredValue::Text { value } => serde_json::Value::String(value),
            StructuredValue::Array { items } => {
                serde_json::Value::Array(items.into_iter().map(Into::into).collect())
            }
            StructuredValue::Object { fields } => serde_json::Value::Object(
                fields
                    .into_iter()
                    .map(|field| (field.name, serde_json::Value::from(field.value)))
                    .collect(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{StructuredObject, StructuredValue};

    #[test]
    fn structured_values_preserve_number_text_and_round_trip() {
        let value = StructuredObject::try_from(serde_json::json!({
            "integer": 1,
            "decimal": 1.25,
            "items": [true, null],
        }))
        .unwrap();

        assert!(matches!(
            value.get("integer"),
            Some(StructuredValue::Integer { value: 1 })
        ));
        assert!(
            matches!(value.get("decimal"), Some(StructuredValue::Number { value }) if value == "1.25")
        );
        assert_eq!(
            serde_json::Value::from(value),
            serde_json::json!({
                "integer": 1,
                "decimal": 1.25,
                "items": [true, null],
            })
        );
    }
}
