//! Declarations for Claude plugin channels and their process option values.

use std::collections::{BTreeMap, BTreeSet};

use agena_domain::claude_channels::{ClaudeChannelOption, ClaudeChannelOptionKind as Kind};
use serde::Deserialize;
use serde_json::Value;

use super::{MAX_COMPONENT_ENTRIES, mcp};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Channel {
    pub(super) server: String,
    display_name: Option<String>,
    #[serde(default)]
    user_config: BTreeMap<String, UserOption>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserOption {
    #[serde(rename = "type")]
    kind: Kind,
    title: String,
    description: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    sensitive: bool,
    #[serde(default)]
    multiple: bool,
    #[serde(default)]
    options: Vec<String>,
    default: Option<Value>,
    min: Option<f64>,
    max: Option<f64>,
}

pub(super) fn discover(
    source: Option<&Value>,
    diagnostics: &mut Vec<String>,
) -> BTreeMap<String, Channel> {
    let Some(source) = source else {
        return BTreeMap::new();
    };
    let Some(values) = source.as_array() else {
        diagnostics.push("Claude channels must be an array".into());
        return BTreeMap::new();
    };
    let mut channels = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for (index, value) in values.iter().take(MAX_COMPONENT_ENTRIES).enumerate() {
        let parsed = serde_json::from_value::<Channel>(value.clone())
            .map_err(|_| "invalid channel fields or userConfig schema")
            .and_then(|channel| {
                channel.validate()?;
                if !seen.insert(channel.server.clone()) {
                    return Err("the MCP server already has a channel declaration");
                }
                Ok(channel)
            });
        match parsed {
            Ok(channel) => {
                channels.insert(channel.server.clone(), channel);
            }
            Err(error) => diagnostics.push(format!("Claude channels[{index}]: {error}")),
        }
    }
    if values.len() > MAX_COMPONENT_ENTRIES {
        diagnostics.push("Too many Claude channel declarations".into());
    }
    channels
}

impl Channel {
    fn validate(&self) -> Result<(), &'static str> {
        if !bounded_label(&self.server, 256)
            || self
                .display_name
                .as_ref()
                .is_some_and(|name| !bounded_label(name, 128))
            || self.user_config.len() > MAX_COMPONENT_ENTRIES
        {
            return Err("invalid server name, displayName or option count");
        }
        for (name, option) in &self.user_config {
            if !mcp::valid_env_name(name) || name.len() > 128 || !option.valid() {
                return Err("invalid channel userConfig option");
            }
        }
        Ok(())
    }

    pub(super) fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.server)
    }

    pub(super) fn options(
        &self,
        environment: &BTreeMap<String, String>,
    ) -> Vec<ClaudeChannelOption> {
        self.user_config
            .iter()
            .map(|(key, option)| ClaudeChannelOption {
                key: key.clone(),
                kind: option.kind,
                title: option.title.clone(),
                description: option.description.clone(),
                required: option.required,
                sensitive: option.sensitive,
                configured: matches!(option.resolve(environment.get(key)), Ok(Some(_))),
                multiple: option.multiple,
                choices: option.options.clone(),
                min: option.min,
                max: option.max,
            })
            .collect()
    }

    /// Render the option values once, keeping secrets out of diagnostics.
    pub(super) fn resolve(
        &self,
        environment: &mut BTreeMap<String, String>,
        options: &mut BTreeMap<String, String>,
        credentials: &mut BTreeMap<String, String>,
    ) -> Result<(), String> {
        for (key, option) in &self.user_config {
            let variable = environment.remove(key);
            let value = option
                .resolve(variable.as_ref())
                .map_err(|reason| format!("Claude channel user_config option {key:?} {reason}"))?;
            if option.required && value.as_deref().is_none_or(str::is_empty) {
                return Err(format!(
                    "Claude channel user_config option {key:?} needs a value"
                ));
            }
            let value = value.unwrap_or_default();
            if option.sensitive && !value.is_empty() {
                credentials.insert(key.clone(), value.clone());
            }
            options.insert(key.clone(), value);
        }
        Ok(())
    }
}

impl UserOption {
    fn valid(&self) -> bool {
        bounded_label(&self.title, 256)
            && bounded_label(&self.description, 2048)
            && (!self.multiple || self.kind == Kind::String)
            && (self.min.is_none() && self.max.is_none() || self.kind == Kind::Number)
            && self.min.zip(self.max).is_none_or(|(min, max)| min <= max)
            && (self.options.is_empty()
                || (self.kind == Kind::String
                    && !self.multiple
                    && !self.sensitive
                    && (self.required || self.default.is_some())
                    && self.options.len() <= MAX_COMPONENT_ENTRIES
                    && self.options.iter().all(|value| bounded_label(value, 64))
                    && self.options.iter().collect::<BTreeSet<_>>().len() == self.options.len()))
            && self
                .default
                .as_ref()
                .is_none_or(|value| self.render(value).is_some())
    }

    fn resolve(&self, variable: Option<&String>) -> Result<Option<String>, &'static str> {
        if let Some(variable) = variable {
            let raw = mcp::read_var(variable).map_err(|_| "needs a service environment value")?;
            if raw.is_empty() && !self.required {
                return Ok(None);
            }
            let value = match self.kind {
                Kind::String if self.multiple => serde_json::from_str(&raw).ok(),
                Kind::Number | Kind::Boolean => serde_json::from_str(raw.trim()).ok(),
                _ => Some(Value::String(raw)),
            };
            return value
                .as_ref()
                .and_then(|value| self.render(value))
                .map(Some)
                .ok_or("does not match its declared type or choices");
        }
        Ok((!self.sensitive)
            .then(|| self.default.as_ref().and_then(|value| self.render(value)))
            .flatten())
    }

    fn render(&self, value: &Value) -> Option<String> {
        match self.kind {
            Kind::String if self.multiple => {
                let items = value.as_array()?;
                if (self.required && items.is_empty())
                    || items.len() > MAX_COMPONENT_ENTRIES
                    || items
                        .iter()
                        .any(|item| item.as_str().is_none_or(|item| !option_text(item)))
                {
                    return None;
                }
                let value = value.to_string();
                option_text(&value).then_some(value)
            }
            Kind::String | Kind::File | Kind::Directory => {
                let value = value.as_str()?;
                (option_text(value)
                    && (!self.required || !value.is_empty())
                    && (self.options.is_empty()
                        || self.options.iter().any(|choice| choice == value)))
                .then(|| value.to_owned())
            }
            Kind::Boolean => value.as_bool().map(|value| value.to_string()),
            Kind::Number => {
                let number = value.as_f64()?;
                (self.min.is_none_or(|min| min <= number)
                    && self.max.is_none_or(|max| number <= max))
                .then(|| value.to_string())
            }
        }
    }
}

fn bounded_label(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control)
}

fn option_text(value: &str) -> bool {
    value.len() <= 64 * 1024 && !value.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_channel_options_are_typed_and_secret_values_stay_private() {
        let mut diagnostics = Vec::new();
        let declarations = json!([
            {"server":"alerts","displayName":"Local alerts","userConfig":{
                "count":{"type":"number","title":"Count","description":"Polling count","default":2,"min":1,"max":5},
                "labels":{"type":"string","title":"Labels","description":"Polling labels","multiple":true,"default":["one","two"]},
                "tone":{"type":"string","title":"Tone","description":"Alert tone","options":["warm","neutral"],"default":"neutral"},
                "credential":{"type":"string","title":"Token","description":"Fixture credential","required":true,"sensitive":true,"default":"discarded-example-secret"}
            }},
            {"server":"typo","newField":true},
            {"server":"invalid","userConfig":{
                "count":{"type":"number","title":"Count","description":"Invalid default","default":100,"max":5}
            }}
        ]);
        let parsed = discover(Some(&declarations), &mut diagnostics);
        assert_eq!(parsed.len(), 1);
        assert_eq!(diagnostics.len(), 2);
        let alerts = &parsed["alerts"];
        assert_eq!(alerts.display_name(), "Local alerts");
        let mut options = BTreeMap::new();
        let mut credentials = BTreeMap::new();
        let mut environment = BTreeMap::new();
        assert!(
            alerts
                .resolve(&mut environment, &mut options, &mut credentials)
                .unwrap_err()
                .contains("needs a value")
        );
        environment.insert("credential".into(), "PATH".into());
        let catalog = serde_json::to_string(&alerts.options(&environment)).unwrap();
        alerts
            .resolve(&mut environment, &mut options, &mut credentials)
            .unwrap();
        assert_eq!(options["count"], "2");
        assert_eq!(options["labels"], r#"["one","two"]"#);
        assert_eq!(options["tone"], "neutral");
        assert_eq!(credentials["credential"], options["credential"]);
        assert!(!catalog.contains(&credentials["credential"]));
        assert!(!catalog.contains("discarded-example-secret"));
        assert!(environment.is_empty());
    }
}
