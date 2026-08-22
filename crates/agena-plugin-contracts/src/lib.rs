//! Dependency-safe wire contracts shared by plugin SDKs, hosts, and surfaces.
//!
//! This crate deliberately does not depend on the SDK, host, runtime, or any
//! renderer.  Settings are represented by a small closed AST rather than by
//! JSON Schema so every host can render and validate the same contract.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const SETTINGS_CONTRACT_VERSION: u32 = 1;
pub const MAX_SETTINGS_DEPTH: usize = 16;
pub const MAX_SETTINGS_NODES: usize = 256;
pub const MAX_SETTINGS_CHILDREN: usize = 128;
pub const MAX_SETTINGS_OPTIONS: usize = 128;
pub const MAX_SETTINGS_VARIANTS: usize = 64;
pub const MAX_SETTINGS_ID_CHARS: usize = 128;
pub const MAX_SETTINGS_TEXT_CHARS: usize = 8_192;
pub const MAX_SETTINGS_DEFAULT_BYTES: usize = 1_048_576;
pub const MAX_JSON_ESCAPE_BYTES: u32 = 1_048_576;
pub const MAX_JSON_ESCAPE_DEPTH: u8 = 12;
pub const MAX_OPERATION_ID_CHARS: usize = 128;
pub const MAX_OPERATION_TEXT_CHARS: usize = 8_192;
pub const MAX_OPERATION_ALIASES: usize = 16;
pub const MAX_OPERATION_DIAGNOSTICS: usize = 32;
pub const MAX_OPERATION_EFFECTS: usize = 8;
pub const MAX_PLUGIN_SERVICES: usize = 128;

/// Tool presentation titles are compact scan labels, not result dumps.
pub const TOOL_TITLE_MAX_DISPLAY_WIDTH: usize = 96;
/// Durable Operation summaries are compact result statements.
pub const TOOL_SUMMARY_MAX_DISPLAY_WIDTH: usize = 120;

/// Normalize a tool/plugin title at the shared contract boundary. This lives
/// in the lightweight contract crate so SDK authors do not need Agena's
/// syntax-tree/runtime tool implementation just to format one-line metadata.
pub fn normalize_tool_title(title: impl AsRef<str>) -> String {
    normalize_tool_presentation_line(title, TOOL_TITLE_MAX_DISPLAY_WIDTH)
}

/// Normalize a tool/plugin summary at the shared contract boundary.
pub fn normalize_tool_summary(summary: impl AsRef<str>) -> String {
    normalize_tool_presentation_line(summary, TOOL_SUMMARY_MAX_DISPLAY_WIDTH)
}

fn normalize_tool_presentation_line(value: impl AsRef<str>, max_width: usize) -> String {
    let normalized = value
        .as_ref()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if UnicodeWidthStr::width(normalized.as_str()) <= max_width {
        return normalized;
    }
    let content_width = max_width.saturating_sub(1);
    let mut width = 0_usize;
    let mut bounded = String::new();
    for grapheme in normalized.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width.saturating_add(grapheme_width) > content_width {
            break;
        }
        bounded.push_str(grapheme);
        width = width.saturating_add(grapheme_width);
    }
    bounded = bounded.trim_end().to_owned();
    bounded.push('…');
    bounded
}

/// Host-neutral syntax error for Agena's stable `namespace.plugin` identity.
/// The SDK, marketplace, runtime configuration and tooling all delegate to
/// this contract so independent tooling never grows a second slug grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginIdentityError {
    MissingSeparator(String),
    InvalidComponent {
        label: &'static str,
        value: String,
        reason: String,
    },
}

impl fmt::Display for PluginIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSeparator(value) => {
                write!(f, "plugin key `{value}` must use `namespace.plugin` format")
            }
            Self::InvalidComponent {
                label,
                value,
                reason,
            } => write!(f, "invalid {label} `{value}`: {reason}"),
        }
    }
}

impl std::error::Error for PluginIdentityError {}

pub fn normalize_plugin_identity_parts(
    namespace: &str,
    name: &str,
) -> Result<(String, String), PluginIdentityError> {
    fn segment(value: &str, label: &'static str) -> Result<String, PluginIdentityError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(PluginIdentityError::InvalidComponent {
                label,
                value: value.to_string(),
                reason: "cannot be empty".to_string(),
            });
        }
        if trimmed.contains('.') {
            return Err(PluginIdentityError::InvalidComponent {
                label,
                value: value.to_string(),
                reason: "must not contain `.`".to_string(),
            });
        }
        Ok(trimmed.to_string())
    }

    Ok((
        segment(namespace, "plugin namespace")?,
        segment(name, "plugin name")?,
    ))
}

pub fn normalize_plugin_identity(value: &str) -> Result<(String, String), PluginIdentityError> {
    let trimmed = value.trim();
    let Some((namespace, name)) = trimmed.split_once('.') else {
        return Err(PluginIdentityError::MissingSeparator(trimmed.to_string()));
    };
    if name.contains('.') {
        return Err(PluginIdentityError::InvalidComponent {
            label: "plugin name",
            value: name.to_string(),
            reason: "must not contain `.`".to_string(),
        });
    }
    normalize_plugin_identity_parts(namespace, name)
}

pub fn validate_plugin_identity(value: &str) -> Result<(), PluginIdentityError> {
    normalize_plugin_identity(value).map(|_| ())
}

/// A complete settings/form contract. The root is normally a fixed object,
/// but the AST also supports a bounded primitive root for small plugins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SettingsContract {
    pub version: u32,
    pub root: SettingsNode,
}

fn validate_operation_slash(value: &str) -> Result<(), OperationDefinitionError> {
    let Some(name) = value.strip_prefix('/') else {
        return Err(OperationDefinitionError::new(
            "operation slash must start with `/`",
        ));
    };
    if name.contains('/') {
        return Err(OperationDefinitionError::new(
            "operation slash must contain exactly one leading `/`",
        ));
    }
    validate_identifier(name, "operation slash").map_err(OperationDefinitionError::from_contract)
}

impl SettingsContract {
    pub fn new(root: SettingsNode) -> Self {
        Self {
            version: SETTINGS_CONTRACT_VERSION,
            root,
        }
    }

    /// Empty closed object contract for no-argument RPC methods and operations.
    pub fn empty_object(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(SettingsNode::root_object(title, description))
    }

    /// Explicit bounded JSON escape hatch for machine-to-machine contracts.
    /// This does not make plugin settings open-ended: authors must opt into a
    /// JSON node and the host still enforces byte/depth limits.
    pub fn bounded_json(
        title: impl Into<String>,
        description: impl Into<String>,
        max_bytes: u32,
        max_depth: u8,
    ) -> Self {
        Self::new(SettingsNode {
            id: "root".to_string(),
            path: String::new(),
            title: title.into(),
            description: description.into(),
            required: true,
            default: None,
            constraints: SettingsConstraints::default(),
            sensitive: false,
            secret: false,
            kind: SettingsNodeKind::Json {
                max_bytes,
                max_depth,
            },
        })
    }

    /// Validate the contract itself before it crosses a plugin boundary.
    pub fn validate(&self) -> Result<(), SettingsContractError> {
        if self.version != SETTINGS_CONTRACT_VERSION {
            return Err(SettingsContractError::new(format!(
                "unsupported settings contract version {}; expected {}",
                self.version, SETTINGS_CONTRACT_VERSION
            )));
        }
        let mut count = 0;
        self.root.validate_at(0, &mut count, "")
    }

    /// Validate one persisted JSON config value against this constrained AST.
    pub fn validate_value(&self, value: &Value) -> Result<(), SettingsValueError> {
        self.validate().map_err(|error| {
            SettingsValueError::at(error.path.unwrap_or_default(), error.message)
        })?;
        validate_value_at(&self.root, value, "")
    }

    /// Materialize a deterministic initial value for editors and invocations.
    /// Explicit node defaults win; otherwise the closed AST supplies a stable
    /// kind-specific seed and recursively fills required/defaulted children.
    pub fn default_value(&self) -> Result<Value, SettingsValueError> {
        self.validate().map_err(|error| {
            SettingsValueError::at(error.path.unwrap_or_default(), error.message)
        })?;
        let value = materialize_node_value(&self.root);
        self.validate_value(&value)?;
        Ok(value)
    }

    /// Parse command-palette or slash shorthand through the same contract used
    /// by every client. Full JSON is always accepted first. A single-field
    /// object accepts a bare scalar; a multi-field object accepts
    /// whitespace-delimited `field=value` tokens. Complex values must use
    /// JSON, keeping quoting and nesting unambiguous and server-owned.
    pub fn parse_shorthand(&self, raw: &str) -> Result<Value, SettingsValueError> {
        self.validate().map_err(|error| {
            SettingsValueError::at(error.path.unwrap_or_default(), error.message)
        })?;
        let raw = raw.trim();
        if raw.is_empty() {
            return self.default_value();
        }

        if let Ok(value) = serde_json::from_str::<Value>(raw) {
            if self.validate_value(&value).is_ok() {
                return Ok(value);
            }
            if let SettingsNodeKind::Object { fields } = &self.root.kind
                && fields.len() == 1
            {
                let wrapped =
                    Value::Object(serde_json::Map::from_iter([(fields[0].id.clone(), value)]));
                self.validate_value(&wrapped)?;
                return Ok(wrapped);
            }
        }

        let value = match &self.root.kind {
            SettingsNodeKind::Object { fields } if fields.len() == 1 => {
                let field = &fields[0];
                let mut object = match materialize_node_value(&self.root) {
                    Value::Object(object) => object,
                    _ => serde_json::Map::new(),
                };
                object.insert(field.id.clone(), parse_node_literal(field, raw)?);
                Value::Object(object)
            }
            SettingsNodeKind::Object { fields } => {
                let mut object = match materialize_node_value(&self.root) {
                    Value::Object(object) => object,
                    _ => serde_json::Map::new(),
                };
                for token in raw.split_whitespace() {
                    let Some((name, literal)) = token.split_once('=') else {
                        return Err(SettingsValueError::at(
                            self.root.path.clone(),
                            "multi-field input must use field=value tokens or full JSON",
                        ));
                    };
                    let Some(field) = fields.iter().find(|field| field.id == name) else {
                        return Err(SettingsValueError::at(
                            join_path(&self.root.path, name),
                            format!("unknown input field `{name}`"),
                        ));
                    };
                    object.insert(field.id.clone(), parse_node_literal(field, literal)?);
                }
                Value::Object(object)
            }
            _ => parse_node_literal(&self.root, raw)?,
        };
        self.validate_value(&value)?;
        Ok(value)
    }
}

/// One bounded settings/form node. The common metadata is intentionally kept
/// outside the tagged kind so Web and TUI renderers do not need kind-specific
/// copies of field identity, labels, defaults, or sensitivity rules.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SettingsNode {
    /// Stable machine identifier within the containing object/variant.
    pub id: String,
    /// Stable JSON-pointer-like path from the settings root. The root path is
    /// the empty string; object fields append `/<field-id>`.
    pub path: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default)]
    pub constraints: SettingsConstraints,
    /// The field may contain credentials or other values that must not be
    /// echoed by a renderer or diagnostic.
    #[serde(default)]
    pub sensitive: bool,
    /// The field is a reference to a host-managed secret, never an inline
    /// secret value. This is distinct from `sensitive`, which can apply to a
    /// non-secret value such as private initialization data.
    #[serde(default)]
    pub secret: bool,
    #[serde(flatten)]
    pub kind: SettingsNodeKind,
}

impl<'de> Deserialize<'de> for SettingsNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let Value::Object(mut object) = value else {
            return Err(serde::de::Error::custom("settings node must be an object"));
        };
        const FIELDS: &[&str] = &[
            "id",
            "path",
            "title",
            "description",
            "required",
            "default",
            "constraints",
            "sensitive",
            "secret",
        ];
        const KIND_FIELDS: &[&str] = &[
            "kind",
            "options",
            "path_kind",
            "fields",
            "item",
            "value",
            "discriminator",
            "variants",
            "max_bytes",
            "max_depth",
        ];
        let unknown = object
            .keys()
            .filter(|key| !FIELDS.contains(&key.as_str()) && !KIND_FIELDS.contains(&key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(serde::de::Error::custom(format!(
                "unknown settings node field `{}`",
                unknown[0]
            )));
        }
        let kind = serde_json::from_value::<SettingsNodeKind>(Value::Object({
            let mut kind_object = object.clone();
            for field in FIELDS {
                kind_object.remove(*field);
            }
            kind_object
        }))
        .map_err(serde::de::Error::custom)?;
        let take = |object: &mut serde_json::Map<String, Value>, key: &str| object.remove(key);
        let id = take(&mut object, "id")
            .ok_or_else(|| serde::de::Error::missing_field("id"))
            .and_then(|value| serde_json::from_value(value).map_err(serde::de::Error::custom))?;
        let path = take(&mut object, "path")
            .ok_or_else(|| serde::de::Error::missing_field("path"))
            .and_then(|value| serde_json::from_value(value).map_err(serde::de::Error::custom))?;
        let title = take(&mut object, "title")
            .ok_or_else(|| serde::de::Error::missing_field("title"))
            .and_then(|value| serde_json::from_value(value).map_err(serde::de::Error::custom))?;
        let description = take(&mut object, "description")
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or_default();
        let required = take(&mut object, "required")
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or(false);
        let default = take(&mut object, "default");
        let constraints = take(&mut object, "constraints")
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or_default();
        let sensitive = take(&mut object, "sensitive")
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or(false);
        let secret = take(&mut object, "secret")
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or(false);
        Ok(Self {
            id,
            path,
            title,
            description,
            required,
            default,
            constraints,
            sensitive,
            secret,
            kind,
        })
    }
}

impl SettingsNode {
    pub fn root_object(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: "root".to_string(),
            path: String::new(),
            title: title.into(),
            description: description.into(),
            required: true,
            default: Some(Value::Object(Default::default())),
            constraints: SettingsConstraints::default(),
            sensitive: false,
            secret: false,
            kind: SettingsNodeKind::Object { fields: Vec::new() },
        }
    }

    fn validate_at(
        &self,
        depth: usize,
        count: &mut usize,
        parent_path: &str,
    ) -> Result<(), SettingsContractError> {
        *count += 1;
        if *count > MAX_SETTINGS_NODES {
            return Err(SettingsContractError::new(format!(
                "settings contract exceeds the {} node limit",
                MAX_SETTINGS_NODES
            )));
        }
        if depth > MAX_SETTINGS_DEPTH {
            return Err(SettingsContractError::at(
                self.path.clone(),
                format!(
                    "settings contract exceeds the {} level depth limit",
                    MAX_SETTINGS_DEPTH
                ),
            ));
        }
        validate_identifier(&self.id, "settings field id")?;
        if self.path.len() > MAX_SETTINGS_ID_CHARS * MAX_SETTINGS_DEPTH {
            return Err(SettingsContractError::at(
                self.path.clone(),
                "settings field path is too long",
            ));
        }
        if self.path != parent_path && depth > 0 && !self.path.starts_with(parent_path) {
            return Err(SettingsContractError::at(
                self.path.clone(),
                "settings field path is not nested under its parent",
            ));
        }
        validate_text(&self.title, "settings field title", MAX_SETTINGS_TEXT_CHARS)?;
        validate_text(
            &self.description,
            "settings field description",
            MAX_SETTINGS_TEXT_CHARS,
        )?;
        self.constraints.validate(&self.kind, &self.path)?;
        if let Some(default) = &self.default {
            let bytes = serde_json::to_vec(default).map_err(|error| {
                SettingsContractError::at(
                    self.path.clone(),
                    agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to encode the settings default value",
                        &error,
                    ),
                )
            })?;
            if bytes.len() > MAX_SETTINGS_DEFAULT_BYTES {
                return Err(SettingsContractError::at(
                    self.path.clone(),
                    "settings default is too large",
                ));
            }
        }

        match &self.kind {
            SettingsNodeKind::Choice { options } | SettingsNodeKind::MultiChoice { options } => {
                if options.is_empty() || options.len() > MAX_SETTINGS_OPTIONS {
                    return Err(SettingsContractError::at(
                        self.path.clone(),
                        format!("settings choice must contain 1..={MAX_SETTINGS_OPTIONS} options"),
                    ));
                }
                validate_options(options, &self.path)?;
            }
            SettingsNodeKind::Object { fields } => {
                if fields.len() > MAX_SETTINGS_CHILDREN {
                    return Err(SettingsContractError::at(
                        self.path.clone(),
                        format!("settings object exceeds the {MAX_SETTINGS_CHILDREN} field limit"),
                    ));
                }
                let mut ids = std::collections::BTreeSet::new();
                for field in fields {
                    if !ids.insert(field.id.clone()) {
                        return Err(SettingsContractError::at(
                            self.path.clone(),
                            format!("duplicate settings field id `{}`", field.id),
                        ));
                    }
                    field.validate_at(depth + 1, count, &self.path)?;
                }
            }
            SettingsNodeKind::List { item } | SettingsNodeKind::Record { value: item } => {
                item.validate_at(depth + 1, count, &self.path)?;
            }
            SettingsNodeKind::TaggedVariant {
                discriminator,
                variants,
            } => {
                validate_identifier(discriminator, "settings discriminator")?;
                if variants.is_empty() || variants.len() > MAX_SETTINGS_VARIANTS {
                    return Err(SettingsContractError::at(
                        self.path.clone(),
                        format!(
                            "settings variant must contain 1..={MAX_SETTINGS_VARIANTS} variants"
                        ),
                    ));
                }
                let mut ids = std::collections::BTreeSet::new();
                for variant in variants {
                    validate_identifier(&variant.id, "settings variant id")?;
                    validate_text(
                        &variant.title,
                        "settings variant title",
                        MAX_SETTINGS_TEXT_CHARS,
                    )?;
                    validate_text(
                        &variant.description,
                        "settings variant description",
                        MAX_SETTINGS_TEXT_CHARS,
                    )?;
                    if !ids.insert(variant.id.clone()) {
                        return Err(SettingsContractError::at(
                            self.path.clone(),
                            format!("duplicate settings variant id `{}`", variant.id),
                        ));
                    }
                    if variant.fields.len() > MAX_SETTINGS_CHILDREN {
                        return Err(SettingsContractError::at(
                            self.path.clone(),
                            format!(
                                "settings variant exceeds the {MAX_SETTINGS_CHILDREN} field limit"
                            ),
                        ));
                    }
                    let mut field_ids = std::collections::BTreeSet::new();
                    for field in &variant.fields {
                        if !field_ids.insert(field.id.clone()) {
                            return Err(SettingsContractError::at(
                                self.path.clone(),
                                format!("duplicate settings variant field id `{}`", field.id),
                            ));
                        }
                        field.validate_at(depth + 1, count, &self.path)?;
                    }
                }
            }
            SettingsNodeKind::Boolean
            | SettingsNodeKind::Text
            | SettingsNodeKind::SecretReference
            | SettingsNodeKind::Integer
            | SettingsNodeKind::Number
            | SettingsNodeKind::Path { .. }
            | SettingsNodeKind::Url
            | SettingsNodeKind::Duration
            | SettingsNodeKind::Json { .. } => {}
        }
        Ok(())
    }
}

/// Closed set of settings node types. No renderer receives JSON Schema
/// composition or traversal primitives.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SettingsNodeKind {
    Boolean,
    Text,
    SecretReference,
    Integer,
    Number,
    Choice {
        options: Vec<SettingsOption>,
    },
    MultiChoice {
        options: Vec<SettingsOption>,
    },
    Path {
        #[serde(default)]
        path_kind: PathInputKind,
    },
    Url,
    Duration,
    Object {
        fields: Vec<SettingsNode>,
    },
    List {
        item: Box<SettingsNode>,
    },
    Record {
        value: Box<SettingsNode>,
    },
    TaggedVariant {
        discriminator: String,
        variants: Vec<SettingsVariant>,
    },
    /// Explicit escape hatch for bounded JSON data. The field must be marked
    /// by the plugin author in the internal schema and carries hard limits.
    Json {
        max_bytes: u32,
        max_depth: u8,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PathInputKind {
    #[default]
    Any,
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SettingsOption {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SettingsVariant {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub tag: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<SettingsNode>,
}

/// Generic bounded constraints. The host interprets only constraints
/// appropriate for each node kind; unknown JSON Schema keywords never cross
/// this boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct SettingsConstraints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusive_minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusive_maximum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiple_of: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// Anchored or unanchored Rust-regex expression applied to text-like
    /// values. The contract validates it once when the manifest is loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_entries: Option<u32>,
}

impl SettingsConstraints {
    fn validate(&self, kind: &SettingsNodeKind, path: &str) -> Result<(), SettingsContractError> {
        for (label, value) in [
            ("minimum", self.minimum),
            ("maximum", self.maximum),
            ("exclusive_minimum", self.exclusive_minimum),
            ("exclusive_maximum", self.exclusive_maximum),
            ("multiple_of", self.multiple_of),
        ] {
            if value.is_some_and(|value| !value.is_finite()) {
                return Err(SettingsContractError::at(
                    path.to_string(),
                    format!("{label} must be finite"),
                ));
            }
        }
        if let (Some(min), Some(max)) = (self.minimum, self.maximum)
            && min > max
        {
            return Err(SettingsContractError::at(
                path.to_string(),
                "minimum must not exceed maximum",
            ));
        }
        if let (Some(min), Some(max)) = (self.min_length, self.max_length)
            && min > max
        {
            return Err(SettingsContractError::at(
                path.to_string(),
                "min_length must not exceed max_length",
            ));
        }
        if let (Some(min), Some(max)) = (self.min_items, self.max_items)
            && min > max
        {
            return Err(SettingsContractError::at(
                path.to_string(),
                "min_items must not exceed max_items",
            ));
        }
        if let Some(pattern) = &self.pattern {
            if pattern.chars().count() > MAX_SETTINGS_TEXT_CHARS {
                return Err(SettingsContractError::at(
                    path.to_string(),
                    "settings text pattern is too long",
                ));
            }
            regex::Regex::new(pattern).map_err(|error| {
                SettingsContractError::at(
                    path.to_string(),
                    format!("settings text pattern is invalid: {error}"),
                )
            })?;
        }
        match kind {
            SettingsNodeKind::Boolean
            | SettingsNodeKind::Choice { .. }
            | SettingsNodeKind::MultiChoice { .. }
            | SettingsNodeKind::TaggedVariant { .. } => {
                if self.minimum.is_some()
                    || self.maximum.is_some()
                    || self.exclusive_minimum.is_some()
                    || self.exclusive_maximum.is_some()
                    || self.multiple_of.is_some()
                    || self.min_length.is_some()
                    || self.max_length.is_some()
                    || self.pattern.is_some()
                {
                    return Err(SettingsContractError::at(
                        path.to_string(),
                        "numeric/text constraints are not valid for this settings node",
                    ));
                }
            }
            SettingsNodeKind::Text
            | SettingsNodeKind::SecretReference
            | SettingsNodeKind::Path { .. }
            | SettingsNodeKind::Url
            | SettingsNodeKind::Duration => {
                if self.minimum.is_some()
                    || self.maximum.is_some()
                    || self.exclusive_minimum.is_some()
                    || self.exclusive_maximum.is_some()
                    || self.multiple_of.is_some()
                {
                    return Err(SettingsContractError::at(
                        path.to_string(),
                        "numeric constraints are not valid for this settings node",
                    ));
                }
            }
            SettingsNodeKind::Integer | SettingsNodeKind::Number => {
                if self.min_length.is_some()
                    || self.max_length.is_some()
                    || self.pattern.is_some()
                    || self.min_items.is_some()
                    || self.max_items.is_some()
                    || self.max_entries.is_some()
                {
                    return Err(SettingsContractError::at(
                        path.to_string(),
                        "collection/text constraints are not valid for this numeric node",
                    ));
                }
            }
            SettingsNodeKind::Object { .. } => {
                if self.min_length.is_some()
                    || self.max_length.is_some()
                    || self.pattern.is_some()
                    || self.min_items.is_some()
                    || self.max_items.is_some()
                {
                    return Err(SettingsContractError::at(
                        path.to_string(),
                        "scalar/array constraints are not valid for an object",
                    ));
                }
            }
            SettingsNodeKind::List { .. } => {
                if self.min_length.is_some()
                    || self.max_length.is_some()
                    || self.pattern.is_some()
                    || self.max_entries.is_some()
                {
                    return Err(SettingsContractError::at(
                        path.to_string(),
                        "text/map constraints are not valid for a list",
                    ));
                }
            }
            SettingsNodeKind::Record { .. } => {
                if self.min_length.is_some()
                    || self.max_length.is_some()
                    || self.pattern.is_some()
                    || self.min_items.is_some()
                    || self.max_items.is_some()
                {
                    return Err(SettingsContractError::at(
                        path.to_string(),
                        "scalar/array constraints are not valid for a record",
                    ));
                }
            }
            SettingsNodeKind::Json { .. } => {
                if self.minimum.is_some()
                    || self.maximum.is_some()
                    || self.exclusive_minimum.is_some()
                    || self.exclusive_maximum.is_some()
                    || self.multiple_of.is_some()
                    || self.min_length.is_some()
                    || self.max_length.is_some()
                    || self.pattern.is_some()
                    || self.min_items.is_some()
                    || self.max_items.is_some()
                    || self.max_entries.is_some()
                {
                    return Err(SettingsContractError::at(
                        path.to_string(),
                        "generic constraints are not valid for a bounded JSON node",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Services exported and imported by one plugin instance. The manifest only
/// describes dependency seams; provider binding and invocation authority stay
/// with the host.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PluginServiceDeclarations {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<PluginServiceExport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<PluginServiceImport>,
}

impl PluginServiceDeclarations {
    pub fn is_empty(&self) -> bool {
        self.exports.is_empty() && self.imports.is_empty()
    }

    pub fn validate(&self) -> Result<(), ServiceDefinitionError> {
        if self.exports.len() > MAX_PLUGIN_SERVICES || self.imports.len() > MAX_PLUGIN_SERVICES {
            return Err(ServiceDefinitionError::new(format!(
                "plugin service declarations exceed the {MAX_PLUGIN_SERVICES} entry limit"
            )));
        }
        let mut exports = std::collections::BTreeSet::new();
        for export in &self.exports {
            export.validate()?;
            if !exports.insert((export.id.clone(), export.api_version)) {
                return Err(ServiceDefinitionError::new(format!(
                    "duplicate service export `{}` at API version {}",
                    export.id, export.api_version
                )));
            }
        }
        let mut imports = std::collections::BTreeSet::new();
        for import in &self.imports {
            import.validate()?;
            if !imports.insert((import.id.clone(), import.api_version)) {
                return Err(ServiceDefinitionError::new(format!(
                    "duplicate service import `{}` at API version {}",
                    import.id, import.api_version
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginServiceExport {
    pub id: String,
    pub api_version: u32,
    /// Closed method catalog. Cross-plugin service calls are never an ambient
    /// stringly-typed RPC surface: the host validates method input and output
    /// against these contracts on every transport.
    pub methods: Vec<PluginServiceMethod>,
}

impl PluginServiceExport {
    pub fn new(id: impl Into<String>, api_version: u32) -> Self {
        Self {
            id: id.into(),
            api_version,
            methods: Vec::new(),
        }
    }

    pub fn with_method(mut self, method: PluginServiceMethod) -> Self {
        self.methods.push(method);
        self
    }

    fn validate(&self) -> Result<(), ServiceDefinitionError> {
        validate_identifier(&self.id, "service id")
            .map_err(ServiceDefinitionError::from_contract)?;
        if self.api_version == 0 {
            return Err(ServiceDefinitionError::new(format!(
                "service export `{}` must use a positive API version",
                self.id
            )));
        }
        if self.methods.is_empty() {
            return Err(ServiceDefinitionError::new(format!(
                "service export `{}` API v{} must declare at least one method",
                self.id, self.api_version
            )));
        }
        if self.methods.len() > MAX_PLUGIN_SERVICES {
            return Err(ServiceDefinitionError::new(format!(
                "service export `{}` API v{} exceeds the {MAX_PLUGIN_SERVICES} method limit",
                self.id, self.api_version
            )));
        }
        let mut methods = std::collections::BTreeSet::new();
        for method in &self.methods {
            method.validate()?;
            if !methods.insert(method.id.clone()) {
                return Err(ServiceDefinitionError::new(format!(
                    "service export `{}` API v{} declares duplicate method `{}`",
                    self.id, self.api_version, method.id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginServiceMethod {
    pub id: String,
    pub input: SettingsContract,
    pub output: SettingsContract,
}

impl PluginServiceMethod {
    pub fn new(id: impl Into<String>, input: SettingsContract, output: SettingsContract) -> Self {
        Self {
            id: id.into(),
            input,
            output,
        }
    }

    pub fn bounded_json(id: impl Into<String>, max_bytes: u32, max_depth: u8) -> Self {
        let id = id.into();
        Self::new(
            id.clone(),
            SettingsContract::bounded_json("Service input", "", max_bytes, max_depth),
            SettingsContract::bounded_json("Service output", "", max_bytes, max_depth),
        )
    }

    fn validate(&self) -> Result<(), ServiceDefinitionError> {
        validate_identifier(&self.id, "service method")
            .map_err(ServiceDefinitionError::from_contract)?;
        self.input.validate().map_err(|error| {
            ServiceDefinitionError::new(format!(
                "service method `{}` has invalid input contract: {error}",
                self.id
            ))
        })?;
        self.output.validate().map_err(|error| {
            ServiceDefinitionError::new(format!(
                "service method `{}` has invalid output contract: {error}",
                self.id
            ))
        })?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginServiceImport {
    pub id: String,
    pub api_version: u32,
    #[serde(default)]
    pub optional: bool,
    /// Optional configured plugin id used to disambiguate multiple providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

impl PluginServiceImport {
    pub fn required(id: impl Into<String>, api_version: u32) -> Self {
        Self {
            id: id.into(),
            api_version,
            optional: false,
            provider: None,
        }
    }

    pub fn optional(id: impl Into<String>, api_version: u32) -> Self {
        Self {
            id: id.into(),
            api_version,
            optional: true,
            provider: None,
        }
    }

    pub fn from_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    fn validate(&self) -> Result<(), ServiceDefinitionError> {
        validate_identifier(&self.id, "service id")
            .map_err(ServiceDefinitionError::from_contract)?;
        if self.api_version == 0 {
            return Err(ServiceDefinitionError::new(format!(
                "service import `{}` must use a positive API version",
                self.id
            )));
        }
        if let Some(provider) = &self.provider {
            validate_identifier(provider, "service provider plugin id")
                .map_err(ServiceDefinitionError::from_contract)?;
        }
        Ok(())
    }
}

/// Invocation routed by the host from one declared consumer to its resolved
/// provider. Plugins cannot choose a provider at call time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginServiceInvokeInput {
    pub service: String,
    pub api_version: u32,
    pub method: String,
    #[serde(default)]
    pub input: Value,
}

impl PluginServiceInvokeInput {
    pub fn validate(&self) -> Result<(), ServiceDefinitionError> {
        validate_identifier(&self.service, "service id")
            .map_err(ServiceDefinitionError::from_contract)?;
        validate_identifier(&self.method, "service method")
            .map_err(ServiceDefinitionError::from_contract)?;
        if self.api_version == 0 {
            return Err(ServiceDefinitionError::new(
                "service invocation must use a positive API version",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginServiceInvokeOutput {
    pub provider: String,
    pub output: Value,
}

/// Server-owned executable target for an operation. A client can select an
/// operation id, but it cannot supply a target or route one operation through
/// another client effect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginOperationTarget {
    Method { handler: String },
    Tool { tool: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginOperationDefinition {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    pub input: SettingsContract,
    #[serde(default)]
    pub discoverability: OperationDiscoverability,
    pub target: PluginOperationTarget,
}

impl PluginOperationDefinition {
    /// Validate the server-owned operation definition before it is published
    /// in a manifest or catalog.
    pub fn validate(&self) -> Result<(), OperationDefinitionError> {
        validate_identifier(&self.id, "operation id")
            .map_err(OperationDefinitionError::from_contract)?;
        validate_text(&self.title, "operation title", MAX_OPERATION_TEXT_CHARS)
            .map_err(OperationDefinitionError::from_contract)?;
        validate_text(
            &self.description,
            "operation description",
            MAX_OPERATION_TEXT_CHARS,
        )
        .map_err(OperationDefinitionError::from_contract)?;
        validate_text(&self.group, "operation group", MAX_OPERATION_TEXT_CHARS)
            .map_err(OperationDefinitionError::from_contract)?;
        if self.group.trim().is_empty() {
            return Err(OperationDefinitionError::new(
                "operation group must not be empty",
            ));
        }
        if let Some(category) = &self.category {
            validate_text(category, "operation category", MAX_OPERATION_TEXT_CHARS)
                .map_err(OperationDefinitionError::from_contract)?;
        }
        if let Some(slash) = &self.slash {
            validate_operation_slash(slash)?;
        }
        if let Some(usage) = &self.usage {
            validate_text(usage, "operation usage", MAX_OPERATION_TEXT_CHARS)
                .map_err(OperationDefinitionError::from_contract)?;
        }
        if self.aliases.len() > MAX_OPERATION_ALIASES {
            return Err(OperationDefinitionError::new(format!(
                "operation has more than {MAX_OPERATION_ALIASES} aliases"
            )));
        }
        let mut aliases = std::collections::BTreeSet::new();
        for alias in &self.aliases {
            validate_identifier(alias, "operation alias")
                .map_err(OperationDefinitionError::from_contract)?;
            if !aliases.insert(alias) || alias == &self.id {
                return Err(OperationDefinitionError::new(
                    "operation aliases must be unique and must not equal the operation id",
                ));
            }
        }
        self.input.validate().map_err(|error| {
            OperationDefinitionError::new(
                agena_failure::diagnostic::format_error_chain_with_context(
                    "invalid plugin operation input contract",
                    &error,
                ),
            )
        })?;
        match &self.target {
            PluginOperationTarget::Method { handler }
            | PluginOperationTarget::Tool { tool: handler } => {
                validate_identifier(handler, "operation target")
                    .map_err(OperationDefinitionError::from_contract)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OperationDiscoverability {
    #[serde(default = "default_true")]
    pub catalog: bool,
    #[serde(default = "default_true")]
    pub command_palette: bool,
    #[serde(default = "default_true")]
    pub slash: bool,
}

impl Default for OperationDiscoverability {
    fn default() -> Self {
        Self {
            catalog: true,
            command_palette: true,
            slash: true,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginOperationInvokeInput {
    pub operation_id: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slash: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginOperationStatus {
    Succeeded,
    Failed,
    Cancelled,
    Unavailable,
    PermissionRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginOperationDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub sensitive: bool,
}

/// The only effects an operation result may request from a host surface. None
/// of these invoke a tool or another operation; execution stays server-side.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginHostEffect {
    Navigate { path: String },
    OpenUrl { url: String },
    InsertPrompt { prompt: String },
    RefreshPluginSurface { plugin_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginOperationResult {
    pub status: PluginOperationStatus,
    pub title: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PluginOperationDiagnostic>,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<PluginHostEffect>,
}

impl PluginOperationResult {
    pub fn succeeded(summary: impl Into<String>) -> Self {
        Self {
            status: PluginOperationStatus::Succeeded,
            title: String::new(),
            summary: summary.into(),
            detail: None,
            output: None,
            diagnostics: Vec::new(),
            retryable: false,
            effects: Vec::new(),
        }
    }

    pub fn failed(summary: impl Into<String>) -> Self {
        let mut result = Self::succeeded(summary);
        result.status = PluginOperationStatus::Failed;
        result
    }

    pub fn unavailable(summary: impl Into<String>) -> Self {
        let mut result = Self::succeeded(summary);
        result.status = PluginOperationStatus::Unavailable;
        result
    }

    pub fn permission_required(summary: impl Into<String>) -> Self {
        let mut result = Self::succeeded(summary);
        result.status = PluginOperationStatus::PermissionRequired;
        result
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_output(mut self, output: Value) -> Self {
        self.output = Some(output);
        self
    }

    pub fn with_effect(mut self, effect: PluginHostEffect) -> Self {
        self.effects.push(effect);
        self
    }

    pub fn validate(&self) -> Result<(), OperationResultError> {
        if self.title.chars().count() > MAX_OPERATION_TEXT_CHARS
            || self.summary.chars().count() > MAX_OPERATION_TEXT_CHARS
            || self
                .detail
                .as_ref()
                .is_some_and(|v| v.chars().count() > MAX_OPERATION_TEXT_CHARS)
        {
            return Err(OperationResultError::new(
                "operation result text is too long",
            ));
        }
        if self.diagnostics.len() > MAX_OPERATION_DIAGNOSTICS {
            return Err(OperationResultError::new("too many operation diagnostics"));
        }
        if self.effects.len() > MAX_OPERATION_EFFECTS {
            return Err(OperationResultError::new("too many operation effects"));
        }
        for diagnostic in &self.diagnostics {
            if diagnostic.code.trim().is_empty() || diagnostic.message.trim().is_empty() {
                return Err(OperationResultError::new(
                    "operation diagnostics require code and message",
                ));
            }
        }
        Ok(())
    }
}

fn validate_options(options: &[SettingsOption], path: &str) -> Result<(), SettingsContractError> {
    let mut ids = std::collections::BTreeSet::new();
    for option in options {
        validate_identifier(&option.id, "settings option id")?;
        validate_text(
            &option.title,
            "settings option title",
            MAX_SETTINGS_TEXT_CHARS,
        )?;
        validate_text(
            &option.description,
            "settings option description",
            MAX_SETTINGS_TEXT_CHARS,
        )?;
        if !ids.insert(option.id.clone()) {
            return Err(SettingsContractError::at(
                path.to_string(),
                format!("duplicate settings option id `{}`", option.id),
            ));
        }
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> Result<(), SettingsContractError> {
    if value.is_empty()
        || value.chars().count() > MAX_SETTINGS_ID_CHARS
        || value.trim() != value
        || value
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || "._-".contains(ch)))
    {
        return Err(SettingsContractError::new(format!(
            "{label} `{value}` is not a stable identifier"
        )));
    }
    Ok(())
}

fn validate_text(value: &str, label: &str, limit: usize) -> Result<(), SettingsContractError> {
    if value.chars().count() > limit {
        return Err(SettingsContractError::new(format!(
            "{label} exceeds {limit} characters"
        )));
    }
    Ok(())
}

fn materialize_node_value(node: &SettingsNode) -> Value {
    match &node.kind {
        SettingsNodeKind::Object { fields } => {
            let mut object = serde_json::Map::new();
            for field in fields {
                if field.required || field.default.is_some() {
                    object.insert(field.id.clone(), materialize_node_value(field));
                }
            }
            if let Some(Value::Object(defaults)) = &node.default {
                for (key, value) in defaults {
                    object.insert(key.clone(), value.clone());
                }
            }
            Value::Object(object)
        }
        SettingsNodeKind::TaggedVariant {
            discriminator,
            variants,
        } => {
            if let Some(default) = &node.default {
                return default.clone();
            }
            let Some(variant) = variants.first() else {
                return Value::Object(Default::default());
            };
            let mut object =
                serde_json::Map::from_iter([(discriminator.clone(), variant.tag.clone())]);
            for field in &variant.fields {
                if field.required || field.default.is_some() {
                    object.insert(field.id.clone(), materialize_node_value(field));
                }
            }
            Value::Object(object)
        }
        _ => node.default.clone().unwrap_or_else(|| match &node.kind {
            SettingsNodeKind::Boolean => Value::Bool(false),
            SettingsNodeKind::Text
            | SettingsNodeKind::SecretReference
            | SettingsNodeKind::Path { .. }
            | SettingsNodeKind::Url
            | SettingsNodeKind::Duration => Value::String(String::new()),
            SettingsNodeKind::Integer => Value::Number(0.into()),
            SettingsNodeKind::Number => Value::Number(0.into()),
            SettingsNodeKind::Choice { options } => options
                .first()
                .map(|option| option.value.clone())
                .unwrap_or(Value::Null),
            SettingsNodeKind::MultiChoice { .. } | SettingsNodeKind::List { .. } => {
                Value::Array(Vec::new())
            }
            SettingsNodeKind::Record { .. } => Value::Object(Default::default()),
            SettingsNodeKind::Json { .. } => Value::Null,
            SettingsNodeKind::Object { .. } | SettingsNodeKind::TaggedVariant { .. } => {
                unreachable!("container nodes are materialized above")
            }
        }),
    }
}

fn parse_node_literal(node: &SettingsNode, raw: &str) -> Result<Value, SettingsValueError> {
    if let Ok(value) = serde_json::from_str::<Value>(raw)
        && validate_value_at(node, &value, node.path.as_str()).is_ok()
    {
        return Ok(value);
    }

    let value = match &node.kind {
        SettingsNodeKind::Boolean => match raw.to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" | "1" => Value::Bool(true),
            "false" | "off" | "no" | "0" => Value::Bool(false),
            _ => {
                return Err(SettingsValueError::at(
                    node.path.clone(),
                    format!("`{raw}` is not a boolean"),
                ));
            }
        },
        SettingsNodeKind::Integer => Value::Number(
            raw.parse::<i64>()
                .map_err(|_| {
                    SettingsValueError::at(node.path.clone(), format!("`{raw}` is not an integer"))
                })?
                .into(),
        ),
        SettingsNodeKind::Number => {
            let number = raw.parse::<f64>().map_err(|_| {
                SettingsValueError::at(node.path.clone(), format!("`{raw}` is not a number"))
            })?;
            Value::Number(serde_json::Number::from_f64(number).ok_or_else(|| {
                SettingsValueError::at(node.path.clone(), "number must be finite")
            })?)
        }
        SettingsNodeKind::Text
        | SettingsNodeKind::SecretReference
        | SettingsNodeKind::Path { .. }
        | SettingsNodeKind::Url
        | SettingsNodeKind::Duration => Value::String(raw.to_string()),
        SettingsNodeKind::Choice { options } => options
            .iter()
            .find(|option| {
                option.id.eq_ignore_ascii_case(raw)
                    || option.title.eq_ignore_ascii_case(raw)
                    || option.value.as_str().is_some_and(|value| value == raw)
            })
            .map(|option| option.value.clone())
            .ok_or_else(|| {
                SettingsValueError::at(
                    node.path.clone(),
                    format!("`{raw}` is not a declared choice"),
                )
            })?,
        SettingsNodeKind::MultiChoice { options } => {
            let mut values = Vec::new();
            for item in raw
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
            {
                let value = options
                    .iter()
                    .find(|option| {
                        option.id.eq_ignore_ascii_case(item)
                            || option.title.eq_ignore_ascii_case(item)
                            || option.value.as_str().is_some_and(|value| value == item)
                    })
                    .map(|option| option.value.clone())
                    .ok_or_else(|| {
                        SettingsValueError::at(
                            node.path.clone(),
                            format!("`{item}` is not a declared choice"),
                        )
                    })?;
                values.push(value);
            }
            Value::Array(values)
        }
        SettingsNodeKind::Object { .. }
        | SettingsNodeKind::List { .. }
        | SettingsNodeKind::Record { .. }
        | SettingsNodeKind::TaggedVariant { .. }
        | SettingsNodeKind::Json { .. } => {
            return Err(SettingsValueError::at(
                node.path.clone(),
                "complex input must be provided as JSON",
            ));
        }
    };
    validate_value_at(node, &value, node.path.as_str())?;
    Ok(value)
}

fn validate_value_at(
    node: &SettingsNode,
    value: &Value,
    path: &str,
) -> Result<(), SettingsValueError> {
    if value.is_null() && !node.required {
        return Ok(());
    }
    let path = if path.is_empty() {
        node.path.as_str()
    } else {
        path
    };
    match &node.kind {
        SettingsNodeKind::Boolean => require_type(value, ValueType::Boolean, path)?,
        SettingsNodeKind::Text
        | SettingsNodeKind::SecretReference
        | SettingsNodeKind::Path { .. }
        | SettingsNodeKind::Url
        | SettingsNodeKind::Duration => require_type(value, ValueType::String, path)?,
        SettingsNodeKind::Integer => {
            let number = value
                .as_i64()
                .ok_or_else(|| SettingsValueError::at(path, "expected integer"))?;
            validate_number_constraints(&node.constraints, number as f64, path)?;
        }
        SettingsNodeKind::Number => {
            let number = value
                .as_f64()
                .ok_or_else(|| SettingsValueError::at(path, "expected number"))?;
            validate_number_constraints(&node.constraints, number, path)?;
        }
        SettingsNodeKind::Choice { options } => {
            if !options.iter().any(|option| option.value == *value) {
                return Err(SettingsValueError::at(
                    path,
                    "value is not one of the declared choices",
                ));
            }
        }
        SettingsNodeKind::MultiChoice { options } => {
            let values = value
                .as_array()
                .ok_or_else(|| SettingsValueError::at(path, "expected array"))?;
            if values.len() > MAX_SETTINGS_OPTIONS {
                return Err(SettingsValueError::at(path, "too many selected choices"));
            }
            for selected in values {
                if !options.iter().any(|option| option.value == *selected) {
                    return Err(SettingsValueError::at(
                        path,
                        "value is not one of the declared choices",
                    ));
                }
            }
        }
        SettingsNodeKind::Object { fields } => {
            let object = value
                .as_object()
                .ok_or_else(|| SettingsValueError::at(path, "expected object"))?;
            for field in fields {
                let child_path = join_path(path, &field.id);
                match object.get(&field.id) {
                    Some(value) => validate_value_at(field, value, &child_path)?,
                    None if field.required => {
                        return Err(SettingsValueError::at(
                            child_path,
                            "required field is missing",
                        ));
                    }
                    None => {}
                }
            }
            if object
                .keys()
                .any(|key| !fields.iter().any(|field| field.id == *key))
            {
                return Err(SettingsValueError::at(
                    path,
                    "object contains an undeclared field",
                ));
            }
        }
        SettingsNodeKind::List { item } => {
            let values = value
                .as_array()
                .ok_or_else(|| SettingsValueError::at(path, "expected array"))?;
            validate_collection_constraints(&node.constraints, values.len(), path)?;
            for (index, value) in values.iter().enumerate() {
                validate_value_at(item, value, &format!("{path}/{index}"))?;
            }
        }
        SettingsNodeKind::Record { value: item } => {
            let object = value
                .as_object()
                .ok_or_else(|| SettingsValueError::at(path, "expected object"))?;
            if let Some(max) = node.constraints.max_entries
                && object.len() > max as usize
            {
                return Err(SettingsValueError::at(
                    path,
                    "record contains too many entries",
                ));
            }
            for (key, value) in object {
                validate_value_at(item, value, &join_path(path, key))?;
            }
        }
        SettingsNodeKind::TaggedVariant {
            discriminator,
            variants,
        } => {
            let object = value
                .as_object()
                .ok_or_else(|| SettingsValueError::at(path, "expected object"))?;
            let tag = object
                .get(discriminator)
                .ok_or_else(|| SettingsValueError::at(path, "variant discriminator is missing"))?;
            let variant = variants
                .iter()
                .find(|variant| variant.tag == *tag)
                .ok_or_else(|| SettingsValueError::at(path, "unknown variant discriminator"))?;
            for field in &variant.fields {
                let child_path = join_path(path, &field.id);
                match object.get(&field.id) {
                    Some(value) => validate_value_at(field, value, &child_path)?,
                    None if field.required => {
                        return Err(SettingsValueError::at(
                            child_path,
                            "required variant field is missing",
                        ));
                    }
                    None => {}
                }
            }
        }
        SettingsNodeKind::Json {
            max_bytes,
            max_depth,
        } => {
            let bytes = serde_json::to_vec(value).map_err(|error| {
                SettingsValueError::at(
                    path,
                    agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to encode the bounded JSON settings value",
                        &error,
                    ),
                )
            })?;
            if bytes.len() > *max_bytes as usize || json_depth(value) > *max_depth as usize {
                return Err(SettingsValueError::at(
                    path,
                    "bounded JSON value exceeds its limits",
                ));
            }
        }
    }
    if let Some(length) = value.as_str().map(str::chars).map(Iterator::count) {
        if node
            .constraints
            .min_length
            .is_some_and(|min| length < min as usize)
            || node
                .constraints
                .max_length
                .is_some_and(|max| length > max as usize)
        {
            return Err(SettingsValueError::at(
                path,
                "text length is outside the declared bounds",
            ));
        }
        if let Some(pattern) = &node.constraints.pattern {
            let regex = regex::Regex::new(pattern).map_err(|error| {
                SettingsValueError::at(path, format!("invalid text pattern: {error}"))
            })?;
            if !regex.is_match(value.as_str().expect("length came from string")) {
                return Err(SettingsValueError::at(
                    path,
                    "text does not match the declared pattern",
                ));
            }
        }
    }
    Ok(())
}

fn require_type(value: &Value, expected: ValueType, path: &str) -> Result<(), SettingsValueError> {
    let matches = match expected {
        ValueType::Boolean => value.is_boolean(),
        ValueType::String => value.is_string(),
    };
    if matches {
        Ok(())
    } else {
        Err(SettingsValueError::at(
            path,
            format!("expected {}", expected.label()),
        ))
    }
}

enum ValueType {
    Boolean,
    String,
}

impl ValueType {
    fn label(&self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::String => "string",
        }
    }
}

fn validate_number_constraints(
    constraints: &SettingsConstraints,
    value: f64,
    path: &str,
) -> Result<(), SettingsValueError> {
    if constraints.minimum.is_some_and(|min| value < min)
        || constraints.maximum.is_some_and(|max| value > max)
        || constraints
            .exclusive_minimum
            .is_some_and(|min| value <= min)
        || constraints
            .exclusive_maximum
            .is_some_and(|max| value >= max)
    {
        return Err(SettingsValueError::at(
            path,
            "number is outside the declared bounds",
        ));
    }
    if let Some(multiple) = constraints.multiple_of
        && multiple > 0.0
        && ((value / multiple).round() - value / multiple).abs() > f64::EPSILON * 16.0
    {
        return Err(SettingsValueError::at(
            path,
            "number is not a declared multiple",
        ));
    }
    Ok(())
}

fn validate_collection_constraints(
    constraints: &SettingsConstraints,
    length: usize,
    path: &str,
) -> Result<(), SettingsValueError> {
    if constraints
        .min_items
        .is_some_and(|min| length < min as usize)
        || constraints
            .max_items
            .is_some_and(|max| length > max as usize)
    {
        return Err(SettingsValueError::at(
            path,
            "collection length is outside the declared bounds",
        ));
    }
    Ok(())
}

fn join_path(parent: &str, child: &str) -> String {
    format!("{parent}/{}", child.replace('~', "~0").replace('/', "~1"))
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => values.iter().map(json_depth).max().unwrap_or(0) + 1,
        Value::Object(values) => values.values().map(json_depth).max().unwrap_or(0) + 1,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => 0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsContractError {
    pub path: Option<String>,
    pub message: String,
}

impl SettingsContractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            path: None,
            message: message.into(),
        }
    }

    fn at(path: String, message: impl Into<String>) -> Self {
        Self {
            path: (!path.is_empty()).then_some(path),
            message: message.into(),
        }
    }
}

impl fmt::Display for SettingsContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{path}: {}", self.message)
        } else {
            f.write_str(&self.message)
        }
    }
}

impl std::error::Error for SettingsContractError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsValueError {
    pub path: Option<String>,
    pub message: String,
}

impl SettingsValueError {
    fn at(path: impl Into<String>, message: impl Into<String>) -> Self {
        let path = path.into();
        Self {
            path: (!path.is_empty()).then_some(path),
            message: message.into(),
        }
    }
}

impl fmt::Display for SettingsValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{path}: {}", self.message)
        } else {
            f.write_str(&self.message)
        }
    }
}

impl std::error::Error for SettingsValueError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationResultError {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationDefinitionError {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDefinitionError {
    pub message: String,
}

impl ServiceDefinitionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn from_contract(error: SettingsContractError) -> Self {
        Self::new(error.to_string())
    }
}

impl fmt::Display for ServiceDefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServiceDefinitionError {}

impl OperationDefinitionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn from_contract(error: SettingsContractError) -> Self {
        Self::new(error.to_string())
    }
}

impl fmt::Display for OperationDefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OperationDefinitionError {}

impl OperationResultError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for OperationResultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OperationResultError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, path: &str, kind: SettingsNodeKind) -> SettingsNode {
        SettingsNode {
            id: id.to_string(),
            path: path.to_string(),
            title: id.to_string(),
            description: String::new(),
            required: true,
            default: None,
            constraints: SettingsConstraints::default(),
            sensitive: false,
            secret: false,
            kind,
        }
    }

    #[test]
    fn plugin_identity_contract_matches_runtime_key_grammar() {
        assert_eq!(
            normalize_plugin_identity(" agena-tools.FileSystem ").unwrap(),
            ("agena-tools".to_string(), "FileSystem".to_string())
        );
        assert!(validate_plugin_identity("agena.fs").is_ok());
        assert!(validate_plugin_identity("agena").is_err());
        assert!(validate_plugin_identity("agena.fs.tools").is_err());
        assert!(normalize_plugin_identity_parts("agena.tools", "fs").is_err());
        assert!(normalize_plugin_identity_parts("agena", "fs.tools").is_err());
    }

    #[test]
    fn bounded_ast_round_trips_and_rejects_unknown_wire_fields() {
        let contract = SettingsContract::new(node(
            "root",
            "",
            SettingsNodeKind::Object {
                fields: vec![node("enabled", "/enabled", SettingsNodeKind::Boolean)],
            },
        ));
        let value = serde_json::to_value(&contract).expect("serialize contract");
        let decoded: SettingsContract = serde_json::from_value(value).expect("decode contract");
        assert_eq!(decoded, contract);
        assert!(
            serde_json::from_value::<SettingsContract>(serde_json::json!({
                "version": 1,
                "root": {"id":"root", "path":"", "title":"Root", "kind":"boolean", "unknown":true}
            }))
            .is_err()
        );
    }

    #[test]
    fn value_validation_enforces_fixed_objects_and_choices() {
        let contract = SettingsContract::new(node(
            "root",
            "",
            SettingsNodeKind::Object {
                fields: vec![node(
                    "mode",
                    "/mode",
                    SettingsNodeKind::Choice {
                        options: vec![SettingsOption {
                            id: "fast".to_string(),
                            title: "Fast".to_string(),
                            description: String::new(),
                            value: Value::String("fast".to_string()),
                        }],
                    },
                )],
            },
        ));
        contract.validate().expect("valid contract");
        contract
            .validate_value(&serde_json::json!({"mode":"fast"}))
            .expect("valid value");
        assert!(
            contract
                .validate_value(&serde_json::json!({"mode":"slow"}))
                .is_err()
        );
        assert!(
            contract
                .validate_value(&serde_json::json!({"mode":"fast","x":1}))
                .is_err()
        );
    }

    #[test]
    fn operation_result_has_no_client_invocation_effects() {
        let result = PluginOperationResult::succeeded("done").with_effect(
            PluginHostEffect::RefreshPluginSurface {
                plugin_id: "example.plugin".to_string(),
            },
        );
        result.validate().expect("valid result");
        let json = serde_json::to_value(result).expect("serialize result");
        assert!(!json.to_string().contains("invoke_tool"));
        assert!(!json.to_string().contains("invoke_command"));
    }

    #[test]
    fn operation_slash_requires_one_leading_separator() {
        let operation = |slash: &str| PluginOperationDefinition {
            id: "example.run".to_string(),
            title: "Run".to_string(),
            description: String::new(),
            group: "command_palette".to_string(),
            category: None,
            slash: Some(slash.to_string()),
            aliases: Vec::new(),
            usage: None,
            input: SettingsContract::new(SettingsNode::root_object("Input", "")),
            discoverability: OperationDiscoverability::default(),
            target: PluginOperationTarget::Method {
                handler: "run".to_string(),
            },
        };

        operation("/example-run")
            .validate()
            .expect("one leading slash is valid");
        assert!(operation("example-run").validate().is_err());
        assert!(operation("//example-run").validate().is_err());
        assert!(operation("/example/run").validate().is_err());
    }

    #[test]
    fn plugin_service_declarations_are_bounded_and_unambiguous() {
        let services = PluginServiceDeclarations {
            exports: vec![
                PluginServiceExport::new("workspace.search", 1)
                    .with_method(PluginServiceMethod::bounded_json("query", 65536, 16)),
            ],
            imports: vec![
                PluginServiceImport::required("index.read", 2).from_provider("example.index"),
                PluginServiceImport::optional("telemetry.observe", 1),
            ],
        };
        services.validate().expect("valid service declarations");
        assert!(!services.is_empty());

        let duplicate = PluginServiceDeclarations {
            exports: vec![
                PluginServiceExport::new("workspace.search", 1)
                    .with_method(PluginServiceMethod::bounded_json("query", 65536, 16)),
                PluginServiceExport::new("workspace.search", 1)
                    .with_method(PluginServiceMethod::bounded_json("query", 65536, 16)),
            ],
            imports: Vec::new(),
        };
        assert!(duplicate.validate().is_err());
        assert!(
            PluginServiceDeclarations {
                exports: vec![
                    PluginServiceExport::new("bad service", 1)
                        .with_method(PluginServiceMethod::bounded_json("query", 65536, 16))
                ],
                imports: Vec::new(),
            }
            .validate()
            .is_err()
        );
        assert!(
            PluginServiceDeclarations {
                exports: vec![PluginServiceExport::new("workspace.empty", 1)],
                imports: Vec::new(),
            }
            .validate()
            .is_err(),
            "a service export without a method catalog is not callable"
        );
        assert!(
            PluginServiceDeclarations {
                exports: vec![
                    PluginServiceExport::new("workspace.duplicate-method", 1)
                        .with_method(PluginServiceMethod::bounded_json("query", 65536, 16))
                        .with_method(PluginServiceMethod::bounded_json("query", 65536, 16)),
                ],
                imports: Vec::new(),
            }
            .validate()
            .is_err(),
            "duplicate service methods must be rejected"
        );
    }

    #[test]
    fn service_invocation_is_provider_neutral_on_the_wire() {
        let input = PluginServiceInvokeInput {
            service: "workspace.search".to_string(),
            api_version: 1,
            method: "query".to_string(),
            input: serde_json::json!({"q":"rust"}),
        };
        input.validate().expect("valid service invocation");
        let encoded = serde_json::to_value(input).expect("serialize service invocation");
        assert!(encoded.get("provider").is_none());
    }

    #[test]
    fn shorthand_parsing_wraps_single_field_and_validates_it() {
        let contract = SettingsContract::new(node(
            "root",
            "",
            SettingsNodeKind::Object {
                fields: vec![node("query", "/query", SettingsNodeKind::Text)],
            },
        ));

        assert_eq!(
            contract
                .parse_shorthand("release checklist")
                .expect("single-field shorthand"),
            serde_json::json!({"query":"release checklist"})
        );
        assert_eq!(
            contract
                .parse_shorthand("\"release checklist\"")
                .expect("JSON scalar shorthand"),
            serde_json::json!({"query":"release checklist"})
        );
    }

    #[test]
    fn shorthand_parsing_supports_named_scalars_and_declared_choices() {
        let mut enabled = node("enabled", "/enabled", SettingsNodeKind::Boolean);
        enabled.default = Some(Value::Bool(true));
        let contract = SettingsContract::new(node(
            "root",
            "",
            SettingsNodeKind::Object {
                fields: vec![
                    node("limit", "/limit", SettingsNodeKind::Integer),
                    enabled,
                    node(
                        "mode",
                        "/mode",
                        SettingsNodeKind::Choice {
                            options: vec![SettingsOption {
                                id: "fast".to_string(),
                                title: "Fast mode".to_string(),
                                description: String::new(),
                                value: Value::String("fast".to_string()),
                            }],
                        },
                    ),
                ],
            },
        ));

        assert_eq!(
            contract
                .parse_shorthand("limit=5 mode=fast")
                .expect("named shorthand"),
            serde_json::json!({"limit":5,"enabled":true,"mode":"fast"})
        );
        assert!(contract.parse_shorthand("limit=nope mode=fast").is_err());
        assert!(contract.parse_shorthand("unknown=1").is_err());
    }

    #[test]
    fn complex_shorthand_requires_json_and_default_materialization_is_deterministic() {
        let contract = SettingsContract::new(node(
            "root",
            "",
            SettingsNodeKind::Object {
                fields: vec![node(
                    "items",
                    "/items",
                    SettingsNodeKind::List {
                        item: Box::new(node("item", "/items/item", SettingsNodeKind::Text)),
                    },
                )],
            },
        ));

        assert_eq!(
            contract.default_value().expect("default value"),
            serde_json::json!({"items":[]})
        );
        assert_eq!(
            contract
                .parse_shorthand(r#"{"items":["a","b"]}"#)
                .expect("full JSON"),
            serde_json::json!({"items":["a","b"]})
        );
        assert!(contract.parse_shorthand("items=a,b").is_err());
    }
}
