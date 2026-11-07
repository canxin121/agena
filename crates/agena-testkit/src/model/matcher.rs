use std::{panic::AssertUnwindSafe, sync::Arc};

use anyhow::{Result, bail};
use serde_json::Value;

use super::{ModelProtocol, ModelRequest, bounded_text, panic_message};

type ConditionCheck = Arc<dyn Fn(&ModelRequest) -> Option<String> + Send + Sync>;

#[derive(Clone)]
struct Condition {
    name: String,
    check: ConditionCheck,
}

pub(super) struct Mismatch {
    pub matched_conditions: usize,
    pub detail: String,
}

/// An AND of request predicates. Use checks for assertions after routing.
///
/// Unmatched requests report the first failed condition of nearby expectations.
/// Header conditions report the header name and presence, never header values.
#[derive(Clone)]
pub struct RequestMatch {
    pub(super) label: String,
    conditions: Vec<Condition>,
}

impl std::fmt::Debug for RequestMatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RequestMatch")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl RequestMatch {
    pub fn new(
        label: impl Into<String>,
        predicate: impl Fn(&ModelRequest) -> bool + Send + Sync + 'static,
    ) -> Self {
        let label = label.into();
        Self {
            conditions: vec![Condition {
                name: label.clone(),
                check: Arc::new(move |request| {
                    (!predicate(request)).then(|| "predicate returned false".into())
                }),
            }],
            label,
        }
    }

    pub fn any() -> Self {
        Self::new("any completion request", |_| true)
    }

    pub fn user_text(marker: impl Into<String>) -> Self {
        let marker = marker.into();
        Self::new(format!("user text contains {marker:?}"), move |request| {
            request.user_text().contains(&marker)
        })
    }

    pub fn tool_output(call_id: impl Into<String>) -> Self {
        let call_id = call_id.into();
        Self::new(format!("tool output for {call_id:?}"), move |request| {
            request.tool_output(&call_id).is_some()
        })
    }

    pub fn and(self, predicate: impl Fn(&ModelRequest) -> bool + Send + Sync + 'static) -> Self {
        let name = format!("custom predicate {}", self.conditions.len() + 1);
        self.and_named(name, predicate)
    }

    /// Name a custom routing condition so a mismatch identifies its purpose.
    pub fn and_named(
        self,
        name: impl Into<String>,
        predicate: impl Fn(&ModelRequest) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.condition(name, move |request| {
            (!predicate(request)).then(|| "predicate returned false".into())
        })
    }

    fn condition(
        mut self,
        name: impl Into<String>,
        check: impl Fn(&ModelRequest) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.conditions.push(Condition {
            name: name.into(),
            check: Arc::new(check),
        });
        self
    }

    pub fn model(self, model: impl Into<String>) -> Self {
        let model = model.into();
        self.condition("model", move |request| {
            (request.decoded_model().as_deref() != Some(model.as_str())).then(|| {
                format!(
                    "expected {:?}, actual {:?}",
                    bounded_text(&model, 160),
                    request
                        .decoded_model()
                        .map(|actual| bounded_text(&actual, 160))
                )
            })
        })
    }

    pub fn protocol(self, protocol: ModelProtocol) -> Self {
        self.condition("protocol", move |request| {
            (request.protocol() != Some(protocol))
                .then(|| format!("expected {protocol:?}, actual {:?}", request.protocol()))
        })
    }

    pub fn streaming(self, streaming: bool) -> Self {
        self.condition("stream", move |request| {
            (request.streaming() != streaming)
                .then(|| format!("expected {streaming}, actual {}", request.streaming()))
        })
    }

    /// Require exactly one URL query parameter with this value. Query values
    /// may contain API keys, so mismatch diagnostics never print either value.
    pub fn query_parameter(self, name: impl Into<String>, expected: impl Into<String>) -> Self {
        let name = name.into();
        let expected = expected.into();
        self.condition(
            format!("query parameter {:?}", bounded_text(&name, 160)),
            move |request| {
                let values = request.query_values(&name);
                if values.len() == 1 && values[0] == expected {
                    None
                } else if values.is_empty() {
                    Some("parameter is absent (value omitted)".into())
                } else if values.len() > 1 {
                    Some(format!(
                        "expected one value; observed {} (values omitted)",
                        values.len()
                    ))
                } else {
                    Some("value differs (query values omitted)".into())
                }
            },
        )
    }

    pub fn declares_function(self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.condition(
            format!("declares_function({:?})", bounded_text(&name, 160)),
            move |request| {
                (!request.declares_function(&name)).then(|| "function is not declared".into())
            },
        )
    }

    pub fn header(self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into().to_ascii_lowercase();
        let value = value.into();
        self.condition(
            format!("header {:?}", bounded_text(&name, 160)),
            move |request| match request.header(&name) {
                Some(actual) if actual == value => None,
                Some(_) => Some("value differs (header values omitted)".into()),
                None => Some("header is absent (expected value omitted)".into()),
            },
        )
    }

    pub fn json_pointer(self, pointer: impl Into<String>, expected: Value) -> Self {
        let pointer = pointer.into();
        self.condition(
            format!("JSON pointer {:?}", bounded_text(&pointer, 160)),
            move |request| {
                let actual = request.body.pointer(&pointer);
                (actual != Some(&expected)).then(|| {
                    format!(
                        "expected {}, actual {}",
                        bounded_text(&expected.to_string(), 160),
                        actual.map_or_else(
                            || "<missing>".into(),
                            |value| bounded_text(&value.to_string(), 160)
                        )
                    )
                })
            },
        )
    }

    pub(super) fn first_mismatch(&self, request: &ModelRequest) -> Result<Option<Mismatch>> {
        for (matched_conditions, condition) in self.conditions.iter().enumerate() {
            match std::panic::catch_unwind(AssertUnwindSafe(|| (condition.check)(request))) {
                Ok(None) => {}
                Ok(Some(detail)) => {
                    return Ok(Some(Mismatch {
                        matched_conditions,
                        detail: format!(
                            "{}: {}",
                            bounded_text(&condition.name, 192),
                            bounded_text(&detail, 384)
                        ),
                    }));
                }
                Err(payload) => bail!(
                    "matcher {:?}, condition {:?} panicked: {}",
                    bounded_text(&self.label, 192),
                    bounded_text(&condition.name, 192),
                    panic_message(payload)
                ),
            }
        }
        Ok(None)
    }
}
