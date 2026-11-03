//! Composable additional system-prompt fragments.
//!
//! A session prompt is assembled from ordered *layers*: global, workspace,
//! matched model routes, session, and the request. Each layer contributes an
//! ordered list of fragments from a shared library, may hide inherited
//! fragments, and may truncate everything broader than itself.
//!
//! This crate is pure: it performs no I/O. Callers supply fragment bodies from
//! files, configuration, or storage.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Largest accepted body for a single fragment.
pub const MAX_FRAGMENT_BYTES: usize = 16 * 1024;
/// Largest accepted assembled prompt contributed by all layers together.
pub const MAX_TOTAL_BYTES: usize = 128 * 1024;

const MAX_ID_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Errors produced while validating or resolving prompt layers.
pub enum PromptError {
    InvalidId(String),
    DuplicateOrder(String),
    DuplicateInline(String),
    UnknownFragment { scope: PromptScope, id: String },
    FragmentTooLarge { id: String, bytes: usize },
    TotalTooLarge { bytes: usize },
}

impl fmt::Display for PromptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(id) => write!(
                formatter,
                "`{id}` is not a valid fragment id: use 1-{MAX_ID_LEN} characters from a-z, 0-9, '.', '_' or '-'"
            ),
            Self::DuplicateOrder(id) => {
                write!(
                    formatter,
                    "fragment `{id}` appears more than once in the layer order"
                )
            }
            Self::DuplicateInline(id) => {
                write!(
                    formatter,
                    "inline fragment `{id}` is defined more than once in this layer"
                )
            }
            Self::UnknownFragment { scope, id } => write!(
                formatter,
                "fragment `{id}` is not available in the {} library",
                scope.as_str()
            ),
            Self::FragmentTooLarge { id, bytes } => write!(
                formatter,
                "fragment `{id}` is {bytes} bytes; the limit is {MAX_FRAGMENT_BYTES}"
            ),
            Self::TotalTooLarge { bytes } => write!(
                formatter,
                "assembled instructions are {bytes} bytes; the limit is {MAX_TOTAL_BYTES}"
            ),
        }
    }
}

impl std::error::Error for PromptError {}

/// A fragment identifier shared by the library, layer order, and UI.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FragmentId(String);

impl FragmentId {
    pub fn parse(text: &str) -> Result<Self, PromptError> {
        let valid = !text.is_empty()
            && text.len() <= MAX_ID_LEN
            && text.chars().all(|ch| {
                ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '.')
            })
            && !text.starts_with('.');
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(PromptError::InvalidId(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FragmentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for FragmentId {
    type Error = PromptError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<FragmentId> for String {
    fn from(value: FragmentId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// Conditions under which a fragment participates in the assembled prompt.
pub struct PromptCondition {
    /// Route patterns such as `anthropic/*` or `local/openai_responses/deepseek/*`.
    pub models: Vec<String>,
    /// Only contribute when the session exposes tools.
    pub requires_tools: Option<bool>,
}

impl PromptCondition {
    pub fn is_empty(&self) -> bool {
        self.models.is_empty() && self.requires_tools.is_none()
    }

    fn matches(&self, route: &PromptRoute, tools_enabled: bool) -> bool {
        if self
            .requires_tools
            .is_some_and(|required| required != tools_enabled)
        {
            return false;
        }
        self.models.is_empty()
            || self
                .models
                .iter()
                .any(|pattern| route_matches(pattern, route))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// One reusable instruction fragment.
pub struct PromptFragment {
    pub id: FragmentId,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub when: PromptCondition,
}

impl PromptFragment {
    pub fn new(id: FragmentId, body: impl Into<String>) -> Self {
        Self {
            id,
            title: String::new(),
            body: body.into(),
            when: PromptCondition::default(),
        }
    }

    pub fn bytes(&self) -> usize {
        self.body.len()
    }

    fn validate(&self) -> Result<(), PromptError> {
        if self.body.len() > MAX_FRAGMENT_BYTES {
            return Err(PromptError::FragmentTooLarge {
                id: self.id.to_string(),
                bytes: self.body.len(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// How a layer combines with the layers broader than itself.
pub enum LayerMode {
    /// Append to everything broader.
    #[default]
    Append,
    /// Discard everything broader before appending this layer.
    Replace,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// One scope's ordered contribution.
pub struct PromptLayer {
    pub mode: LayerMode,
    /// Fragment ids in render order.
    pub order: Vec<FragmentId>,
    /// Fragments inherited from broader scopes that this layer hides.
    pub disabled: Vec<FragmentId>,
    /// Fragments defined for this scope only.
    pub inline: Vec<PromptFragment>,
}

impl PromptLayer {
    pub fn is_empty(&self) -> bool {
        self.order.is_empty() && self.disabled.is_empty() && self.inline.is_empty()
    }

    pub fn validate(&self) -> Result<(), PromptError> {
        let mut seen = BTreeSet::new();
        for id in &self.order {
            if !seen.insert(id) {
                return Err(PromptError::DuplicateOrder(id.to_string()));
            }
        }
        let mut inline = BTreeSet::new();
        for fragment in &self.inline {
            if !inline.insert(fragment.id.clone()) {
                return Err(PromptError::DuplicateInline(fragment.id.to_string()));
            }
            fragment.validate()?;
        }
        for id in &self.disabled {
            if self.order.contains(id) || inline.contains(id) {
                return Err(PromptError::DuplicateOrder(id.to_string()));
            }
        }
        Ok(())
    }

    fn inline_fragment(&self, id: &FragmentId) -> Option<&PromptFragment> {
        self.inline.iter().find(|fragment| &fragment.id == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Where a layer's instructions came from.
pub enum PromptScope {
    Global,
    Workspace,
    Model,
    Session,
    Request,
}

impl PromptScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Workspace => "workspace",
            Self::Model => "model",
            Self::Session => "session",
            Self::Request => "request",
        }
    }
}

/// The provider/adapter/model route a request executes on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRoute {
    pub provider: String,
    pub adapter: Option<String>,
    pub model: String,
}

impl PromptRoute {
    pub fn new(
        provider: impl Into<String>,
        adapter: Option<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            adapter,
            model: model.into(),
        }
    }
}

impl fmt::Display for PromptRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.adapter {
            Some(adapter) => write!(formatter, "{}/{adapter}/{}", self.provider, self.model),
            None => write!(formatter, "{}/{}", self.provider, self.model),
        }
    }
}

/// Values substituted into fragment bodies.
#[derive(Debug, Clone, Default)]
pub struct PromptContext {
    pub route: Option<PromptRoute>,
    pub workspace: String,
    pub platform: String,
    pub session_id: i64,
    pub tools_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptWarningKind {
    ConditionNotMatched,
    UnknownVariable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A fragment that was skipped or partially substituted.
pub struct PromptWarning {
    pub scope: PromptScope,
    pub id: FragmentId,
    pub kind: PromptWarningKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One fragment in the assembled prompt.
pub struct ResolvedFragment {
    pub id: FragmentId,
    pub title: String,
    pub scope: PromptScope,
    pub position: usize,
    pub bytes: usize,
    pub sha256: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
/// The assembled additional instructions plus provenance.
pub struct PromptResolution {
    pub text: String,
    pub fragments: Vec<ResolvedFragment>,
    pub warnings: Vec<PromptWarning>,
    pub total_bytes: usize,
    pub over_budget: bool,
}

/// One layer and the fragment bodies it can reference.
pub struct LayerSource<'a> {
    pub scope: PromptScope,
    pub layer: &'a PromptLayer,
    pub library: &'a BTreeMap<FragmentId, PromptFragment>,
}

impl<'a> LayerSource<'a> {
    pub fn new(
        scope: PromptScope,
        layer: &'a PromptLayer,
        library: &'a BTreeMap<FragmentId, PromptFragment>,
    ) -> Self {
        Self {
            scope,
            layer,
            library,
        }
    }
}

/// Resolve layers from broadest to narrowest into one ordered prompt.
pub fn resolve(
    layers: &[LayerSource<'_>],
    context: &PromptContext,
) -> Result<PromptResolution, PromptError> {
    let mut resolved: Vec<(FragmentId, ResolvedFragment)> = Vec::new();
    let mut warnings = Vec::new();
    let mut suppressed = BTreeSet::new();

    for source in layers {
        source.layer.validate()?;
        if source.layer.mode == LayerMode::Replace {
            resolved.clear();
        }
        for id in &source.layer.disabled {
            suppressed.insert(id.clone());
            resolved.retain(|(existing, _)| existing != id);
        }
        for id in &source.layer.order {
            if suppressed.contains(id) {
                continue;
            }
            let fragment = source
                .layer
                .inline_fragment(id)
                .or_else(|| source.library.get(id))
                .ok_or_else(|| PromptError::UnknownFragment {
                    scope: source.scope,
                    id: id.to_string(),
                })?;
            fragment.validate()?;
            if !fragment.when.matches(
                context.route.as_ref().unwrap_or(&empty_route()),
                context.tools_enabled,
            ) {
                warnings.push(PromptWarning {
                    scope: source.scope,
                    id: id.clone(),
                    kind: PromptWarningKind::ConditionNotMatched,
                    detail: if fragment.when.models.is_empty() {
                        "requires_tools".to_owned()
                    } else {
                        fragment.when.models.join(", ")
                    },
                });
                continue;
            }
            let (text, unknown) = render(&fragment.body, context);
            if !unknown.is_empty() {
                warnings.push(PromptWarning {
                    scope: source.scope,
                    id: id.clone(),
                    kind: PromptWarningKind::UnknownVariable,
                    detail: unknown.join(", "),
                });
            }
            resolved.retain(|(existing, _)| existing != id);
            resolved.push((
                id.clone(),
                ResolvedFragment {
                    id: id.clone(),
                    title: fragment.title.clone(),
                    scope: source.scope,
                    position: 0,
                    bytes: text.len(),
                    sha256: sha256_hex(text.as_bytes()),
                    text,
                },
            ));
        }
    }

    let mut fragments = Vec::with_capacity(resolved.len());
    for (position, (_, mut fragment)) in resolved.into_iter().enumerate() {
        fragment.position = position;
        fragments.push(fragment);
    }
    let text = fragments
        .iter()
        .map(|fragment| fragment.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let total_bytes = text.len();
    Ok(PromptResolution {
        text,
        fragments,
        warnings,
        total_bytes,
        over_budget: total_bytes > MAX_TOTAL_BYTES,
    })
}

fn empty_route() -> PromptRoute {
    PromptRoute {
        provider: String::new(),
        adapter: None,
        model: String::new(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// True when `pattern` matches `route`; `*` matches any run of characters.
///
/// Patterns match the full `provider/adapter/model` string or the shorter
/// `provider/model` form, so a pattern may skip the adapter segment.
pub fn route_matches(pattern: &str, route: &PromptRoute) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    let full = route.to_string();
    let short = format!("{}/{}", route.provider, route.model);
    glob_match(pattern, &full) || glob_match(pattern, &short)
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

/// Substitute `{{name}}` placeholders; unknown names are reported and kept.
pub fn render(body: &str, context: &PromptContext) -> (String, Vec<String>) {
    let mut unknown = Vec::new();
    let mut output = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            output.push_str(&rest[start..]);
            return (output, unknown);
        };
        let name = after[..end].trim();
        let value = variable(name, context);
        match value {
            Some(value) => output.push_str(&value),
            None => {
                unknown.push(name.to_owned());
                output.push_str(&rest[start..start + 2 + end + 2]);
            }
        }
        rest = &after[end + 2..];
    }
    output.push_str(rest);
    (output, unknown)
}

fn variable(name: &str, context: &PromptContext) -> Option<String> {
    let route = context.route.as_ref();
    match name {
        "model" => Some(route?.model.clone()),
        "provider" => Some(route?.provider.clone()),
        "adapter" => Some(route?.adapter.clone().unwrap_or_default()),
        "workspace" => Some(context.workspace.clone()),
        "platform" => Some(context.platform.clone()),
        "session_id" => Some(context.session_id.to_string()),
        _ => None,
    }
}

/// Names accepted by [`render`]; used by editors to offer completions.
pub const VARIABLES: [&str; 6] = [
    "model",
    "provider",
    "adapter",
    "workspace",
    "platform",
    "session_id",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> FragmentId {
        FragmentId::parse(name).unwrap()
    }

    fn fragment(name: &str, body: &str) -> PromptFragment {
        PromptFragment::new(id(name), body)
    }

    fn library(items: &[PromptFragment]) -> BTreeMap<FragmentId, PromptFragment> {
        items
            .iter()
            .map(|fragment| (fragment.id.clone(), fragment.clone()))
            .collect()
    }

    fn layer(items: &[&str]) -> PromptLayer {
        PromptLayer {
            mode: LayerMode::Append,
            order: items.iter().map(|name| id(name)).collect(),
            disabled: Vec::new(),
            inline: Vec::new(),
        }
    }

    fn context() -> PromptContext {
        PromptContext {
            route: Some(PromptRoute::new(
                "local",
                Some("openai_responses".into()),
                "deepseek/v4",
            )),
            workspace: "/work".to_owned(),
            platform: "darwin".to_owned(),
            session_id: 7,
            tools_enabled: true,
        }
    }

    #[test]
    fn ids_are_restricted_and_serialized_transparently() {
        assert!(FragmentId::parse("house-style.v2").is_ok());
        for invalid in ["", "House", "with space", ".hidden", &"x".repeat(65)] {
            assert!(FragmentId::parse(invalid).is_err(), "{invalid}");
        }
        let json = serde_json::to_value(fragment("style", "Body")).unwrap();
        assert_eq!(json["id"], "style");
    }

    #[test]
    fn broader_layers_render_first_and_narrower_layers_append() {
        let global = library(&[fragment("a", "global a")]);
        let workspace = library(&[fragment("b", "workspace b")]);
        let global_layer = layer(&["a"]);
        let workspace_layer = layer(&["b"]);
        let layers = [
            LayerSource::new(PromptScope::Global, &global_layer, &global),
            LayerSource::new(PromptScope::Workspace, &workspace_layer, &workspace),
        ];
        let resolution = resolve(&layers, &context()).unwrap();
        assert_eq!(resolution.text, "global a\n\nworkspace b");
        assert_eq!(resolution.fragments[0].scope, PromptScope::Global);
        assert_eq!(resolution.fragments[1].position, 1);
    }

    #[test]
    fn replace_truncates_broader_layers_and_disabled_hides_inherited() {
        let global = library(&[fragment("a", "global a"), fragment("b", "global b")]);
        let global_layer = layer(&["a", "b"]);
        let session = PromptLayer {
            mode: LayerMode::Replace,
            order: vec![id("b")],
            disabled: Vec::new(),
            inline: Vec::new(),
        };
        let layers = [
            LayerSource::new(PromptScope::Global, &global_layer, &global),
            LayerSource::new(PromptScope::Session, &session, &global),
        ];
        assert_eq!(resolve(&layers, &context()).unwrap().text, "global b");

        let hidden = PromptLayer {
            mode: LayerMode::Append,
            order: Vec::new(),
            disabled: vec![id("a")],
            inline: Vec::new(),
        };
        let layers = [
            LayerSource::new(PromptScope::Global, &global_layer, &global),
            LayerSource::new(PromptScope::Session, &hidden, &global),
        ];
        let resolution = resolve(&layers, &context()).unwrap();
        assert_eq!(resolution.text, "global b");
    }

    #[test]
    fn narrower_layers_override_the_same_fragment_id() {
        let global = library(&[fragment("style", "broad")]);
        let mut narrow = BTreeMap::new();
        narrow.insert(id("style"), fragment("style", "inline"));
        let global_layer = layer(&["style"]);
        let narrow_layer = layer(&["style"]);
        let layers = [
            LayerSource::new(PromptScope::Global, &global_layer, &global),
            LayerSource::new(PromptScope::Session, &narrow_layer, &narrow),
        ];
        let resolution = resolve(&layers, &context()).unwrap();
        assert_eq!(resolution.text, "inline");
        assert_eq!(resolution.fragments.len(), 1);
        assert_eq!(resolution.fragments[0].scope, PromptScope::Session);
    }

    #[test]
    fn conditions_skip_unmatched_fragments_with_warnings() {
        let mut model_only = fragment("m", "model only");
        model_only.when.models = vec!["anthropic/*".to_owned()];
        let library = library(&[model_only]);
        let model_layer = layer(&["m"]);
        let resolution = resolve(
            &[LayerSource::new(PromptScope::Model, &model_layer, &library)],
            &context(),
        )
        .unwrap();
        assert!(resolution.text.is_empty());
        assert_eq!(
            resolution.warnings[0].kind,
            PromptWarningKind::ConditionNotMatched
        );
    }

    #[test]
    fn route_patterns_match_provider_adapter_and_model() {
        let route = PromptRoute::new("local", Some("openai_responses".into()), "deepseek/v4");
        assert!(route_matches("local/openai_responses/deepseek/v4", &route));
        assert!(route_matches("local/*/deepseek/*", &route));
        assert!(route_matches("local/deepseek/v4", &route));
        assert!(route_matches("*", &route));
        assert!(!route_matches("anthropic/*", &route));
        assert!(!route_matches("local/openai_responses/gpt", &route));
    }

    #[test]
    fn variables_render_and_unknown_names_warn() {
        let library = library(&[fragment(
            "v",
            "{{provider}}/{{adapter}}/{{model}} in {{workspace}} on {{platform}} #{{session_id}} {{nope}}",
        )]);
        let variables = layer(&["v"]);
        let resolution = resolve(
            &[LayerSource::new(PromptScope::Global, &variables, &library)],
            &context(),
        )
        .unwrap();
        assert_eq!(
            resolution.text,
            "local/openai_responses/deepseek/v4 in /work on darwin #7 {{nope}}"
        );
        assert_eq!(resolution.warnings[0].detail, "nope");
    }

    #[test]
    fn oversized_fragments_and_duplicates_are_rejected() {
        let huge = fragment("huge", &"x".repeat(MAX_FRAGMENT_BYTES + 1));
        let library = library(&[huge]);
        let huge_layer = layer(&["huge"]);
        let error = resolve(
            &[LayerSource::new(PromptScope::Global, &huge_layer, &library)],
            &context(),
        )
        .unwrap_err();
        assert!(matches!(error, PromptError::FragmentTooLarge { .. }));

        let duplicate = PromptLayer {
            mode: LayerMode::Append,
            order: vec![id("a"), id("a")],
            disabled: Vec::new(),
            inline: Vec::new(),
        };
        assert!(matches!(
            duplicate.validate(),
            Err(PromptError::DuplicateOrder(_))
        ));
    }

    #[test]
    fn unknown_order_entries_are_actionable() {
        let empty = BTreeMap::new();
        let missing = layer(&["missing"]);
        let error = resolve(
            &[LayerSource::new(PromptScope::Workspace, &missing, &empty)],
            &context(),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "fragment `missing` is not available in the workspace library"
        );
    }

    #[test]
    fn inline_fragments_win_over_the_library_without_order_entries() {
        let mut inline = layer(&[]);
        inline.inline = vec![fragment("only", "inline body")];
        inline.order = vec![id("only")];
        let library = library(&[fragment("only", "library body")]);
        let resolution = resolve(
            &[LayerSource::new(PromptScope::Session, &inline, &library)],
            &context(),
        )
        .unwrap();
        assert_eq!(resolution.text, "inline body");
    }

    #[test]
    fn resolution_round_trips_through_the_preview_contract() {
        let mut model_only = fragment("m", "model only");
        model_only.when.models = vec!["anthropic/*".to_owned()];
        let library = library(&[fragment("body", "inline body"), model_only]);
        let mut conditional = layer(&["body", "m"]);
        conditional.inline = vec![fragment("body", "inline body")];
        let resolution = resolve(
            &[LayerSource::new(
                PromptScope::Session,
                &conditional,
                &library,
            )],
            &context(),
        )
        .unwrap();
        assert_eq!(resolution.warnings.len(), 1);

        // The preview endpoint sends this value as JSON and every client
        // decodes it back, so both directions are part of the contract.
        let json = serde_json::to_value(&resolution).unwrap();
        let decoded: PromptResolution = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, resolution);
    }
}
