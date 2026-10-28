//! Bounded declarative UI programs shared by the browser, terminal and authoring
//! tools. Expressions are data: no JavaScript, templates, URLs or host callbacks.
mod expression;
mod presentation;
#[cfg(test)]
mod tests;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub type UiState = BTreeMap<String, Value>;
/// Canonical operator catalog, shared by the validator and authoring guide.
pub fn ui_supported_operators() -> &'static [&'static str] {
    expression::OPERATORS
}

pub const UI_MAX_NODES: usize = 128;
pub const UI_MAX_EVALUATION_STEPS: usize = 30_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiProgram {
    pub root: String,
    pub nodes: Vec<UiNode>,
    #[serde(default)]
    pub data: UiState,
    #[serde(default)]
    pub state: BTreeMap<String, UiStateField>,
    #[serde(default)]
    pub computed: BTreeMap<String, Value>,
    #[serde(default)]
    pub actions: BTreeMap<String, Vec<UiAction>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiNode {
    pub id: String,
    pub kind: UiKind,
    #[serde(default)]
    pub props: BTreeMap<String, Value>,
    #[serde(default)]
    pub children: Vec<String>,
    /// A typed local state field. Controls write only this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    /// Boolean expression; hidden subtrees do not render or accept events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Value>,
    /// Explicitly declared local action; not an arbitrary tool or URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UiKind {
    Stack,
    Row,
    Grid,
    Card,
    Text,
    Heading,
    Metric,
    Badge,
    Divider,
    Tabs,
    Select,
    Checkbox,
    Switch,
    Slider,
    Number,
    Input,
    Button,
    Progress,
    Chart,
    Table,
    Tasks,
    Timeline,
    Callout,
    Details,
}
impl UiKind {
    /// Canonical component catalog; guide field lists use allowed_props().
    pub const fn all() -> &'static [Self] {
        &[
            Self::Stack,
            Self::Row,
            Self::Grid,
            Self::Card,
            Self::Text,
            Self::Heading,
            Self::Metric,
            Self::Badge,
            Self::Divider,
            Self::Tabs,
            Self::Select,
            Self::Checkbox,
            Self::Switch,
            Self::Slider,
            Self::Number,
            Self::Input,
            Self::Button,
            Self::Progress,
            Self::Chart,
            Self::Table,
            Self::Tasks,
            Self::Timeline,
            Self::Callout,
            Self::Details,
        ]
    }
    pub fn is_container(self) -> bool {
        matches!(
            self,
            Self::Stack | Self::Row | Self::Grid | Self::Card | Self::Details
        )
    }
    pub fn is_control(self) -> bool {
        matches!(
            self,
            Self::Tabs
                | Self::Select
                | Self::Checkbox
                | Self::Switch
                | Self::Slider
                | Self::Number
                | Self::Input
                | Self::Tasks
                | Self::Button
        )
    }
    pub fn allowed_props(self) -> &'static [&'static str] {
        match self {
            Self::Stack | Self::Row | Self::Grid | Self::Card => {
                &["title", "description", "columns", "gap", "tone"]
            }
            Self::Text | Self::Heading => &["text", "tone", "size"],
            Self::Metric => &[
                "label", "value", "prefix", "suffix", "detail", "trend", "tone",
            ],
            Self::Badge => &["text", "tone"],
            Self::Divider => &[],
            Self::Tabs | Self::Select => &["label", "description", "disabled"],
            Self::Checkbox | Self::Switch => &["label", "description", "disabled"],
            Self::Slider | Self::Number => {
                &["label", "description", "prefix", "suffix", "disabled"]
            }
            Self::Input => &["label", "description", "placeholder", "disabled"],
            Self::Button => &["label", "tone", "disabled"],
            Self::Progress => &["label", "value", "max", "detail", "tone"],
            Self::Chart => &[
                "title",
                "chart_type",
                "labels",
                "series",
                "x_label",
                "y_label",
            ],
            Self::Table => &["title", "columns", "rows", "empty"],
            Self::Tasks => &["title", "items", "disabled", "empty"],
            Self::Timeline => &["title", "items"],
            Self::Callout => &["title", "text", "tone"],
            Self::Details => &["title"],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiOption {
    pub value: String,
    pub label: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UiStateKind {
    Boolean,
    Number,
    Text,
    Choice,
    Selection,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiStateField {
    pub kind: UiStateKind,
    pub initial: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<serde_json::Number>,
    #[serde(default)]
    pub options: Vec<UiOption>,
    #[serde(default = "default_text_limit")]
    pub max_length: usize,
}
fn default_text_limit() -> usize {
    1024
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiAction {
    Set { key: String, value: Value },
    Toggle { key: String },
    Reset,
    Copy { text: Value },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiResolvedNode {
    pub id: String,
    pub kind: UiKind,
    pub props: BTreeMap<String, Value>,
    pub children: Vec<UiResolvedNode>,
    pub bind: Option<String>,
    pub value: Value,
    pub field: Option<UiStateField>,
    pub action: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct UiView {
    pub root: UiResolvedNode,
    pub state: UiState,
    pub computed: UiState,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiActionResult {
    pub state: UiState,
    pub copy: Option<String>,
}

pub fn ui_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && !matches!(value, "__proto__" | "prototype" | "constructor")
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}
fn safe_value(value: &Value, depth: usize, count: &mut usize) -> Result<(), String> {
    *count += 1;
    if *count > 12_000 || depth > 24 {
        return Err("UI value complexity budget exceeded".into());
    }
    match value {
        Value::String(s) => super::text(s, "UI text", 6000, false),
        Value::Number(n) => {
            if n.as_f64()
                .is_some_and(|v| v.is_finite() && v.abs() <= 1e100)
            {
                Ok(())
            } else {
                Err("UI number is out of bounds".into())
            }
        }
        Value::Array(values) => {
            if values.len() > 1024 {
                return Err("UI array exceeds 1024 items".into());
            }
            for v in values {
                safe_value(v, depth + 1, count)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            if values.len() > 128 {
                return Err("UI object exceeds 128 fields".into());
            }
            for (k, v) in values {
                if matches!(k.as_str(), "__proto__" | "prototype" | "constructor") {
                    return Err("Unsafe UI property".into());
                }
                super::text(k, "UI key", 120, true)?;
                safe_value(v, depth + 1, count)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
impl UiStateField {
    pub fn accepts(&self, value: &Value) -> bool {
        match self.kind {
            UiStateKind::Boolean => value.is_boolean(),
            UiStateKind::Number => value.as_f64().is_some_and(|n| {
                n.is_finite()
                    && n.abs() <= 1e100
                    && self
                        .min
                        .as_ref()
                        .and_then(|v| v.as_f64())
                        .is_none_or(|min| n >= min)
                    && self
                        .max
                        .as_ref()
                        .and_then(|v| v.as_f64())
                        .is_none_or(|max| n <= max)
            }),
            UiStateKind::Text => value.as_str().is_some_and(|s| {
                s.chars().count() <= self.max_length
                    && super::text(s, "state", self.max_length, false).is_ok()
            }),
            UiStateKind::Choice => value
                .as_str()
                .is_some_and(|s| self.options.iter().any(|o| o.value == s)),
            UiStateKind::Selection => value.as_array().is_some_and(|items| {
                items.len() <= 256
                    && items.iter().all(|v| v.as_str().is_some_and(ui_identifier))
                    && items
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == items.len()
            }),
        }
    }
}
impl UiProgram {
    pub fn reconcile_state(&self, previous: &UiState) -> UiState {
        self.state
            .iter()
            .map(|(key, field)| {
                (
                    key.clone(),
                    previous
                        .get(key)
                        .filter(|v| field.accepts(v))
                        .unwrap_or(&field.initial)
                        .clone(),
                )
            })
            .collect()
    }
    pub fn validate(&self) -> Result<(), String> {
        if !ui_identifier(&self.root) || self.nodes.is_empty() || self.nodes.len() > UI_MAX_NODES {
            return Err("program: need a root and 1..128 nodes".into());
        }
        if self.state.len() > 48
            || self.computed.len() > 64
            || self.actions.len() > 64
            || self.data.len() > 64
        {
            return Err("program namespace budget exceeded".into());
        }
        let value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        safe_value(&value, 0, &mut 0)?;
        let mut ids = BTreeSet::new();
        for (i, node) in self.nodes.iter().enumerate() {
            let path = format!("nodes[{i}]");
            if !ui_identifier(&node.id) || !ids.insert(node.id.as_str()) {
                return Err(format!("{path}.id: invalid or duplicate id"));
            }
            if node.children.len() > 48 || (!node.kind.is_container() && !node.children.is_empty())
            {
                return Err(format!(
                    "{path}.children: only containers may have up to 48 child IDs"
                ));
            }
            for (key, value) in &node.props {
                if !node.kind.allowed_props().contains(&key.as_str()) {
                    return Err(format!(
                        "{path}.props.{key}: unsupported property for this kind"
                    ));
                }
                expression::validate(value, 0).map_err(|e| format!("{path}.props.{key}: {e}"))?;
            }
            if let Some(value) = &node.visible {
                expression::validate(value, 0).map_err(|e| format!("{path}.visible: {e}"))?;
            }
            if node.kind.is_control() && node.kind != UiKind::Button && node.bind.is_none() {
                return Err(format!("{path}.bind: control requires state binding"));
            }
            if let Some(key) = &node.bind {
                let field = self
                    .state
                    .get(key)
                    .ok_or_else(|| format!("{path}.bind: unknown state {key}"))?;
                let expected = match node.kind {
                    UiKind::Tabs | UiKind::Select => UiStateKind::Choice,
                    UiKind::Checkbox | UiKind::Switch => UiStateKind::Boolean,
                    UiKind::Slider | UiKind::Number => UiStateKind::Number,
                    UiKind::Input => UiStateKind::Text,
                    UiKind::Tasks => UiStateKind::Selection,
                    _ => return Err(format!("{path}.bind: only input controls can bind state")),
                };
                if field.kind != expected {
                    return Err(format!("{path}.bind: state type mismatch"));
                }
            }
            if let Some(action) = &node.action
                && (node.kind != UiKind::Button || !self.actions.contains_key(action))
            {
                return Err(format!("{path}.action: button requires a declared action"));
            }
            if node.kind == UiKind::Button && node.action.is_none() {
                return Err(format!("{path}.action: button requires action"));
            }
        }
        for (key, field) in &self.state {
            if !ui_identifier(key)
                || !field.accepts(&field.initial)
                || field.max_length > 6000
                || field.options.len() > 64
            {
                return Err(format!("state.{key}: invalid declaration or default"));
            }
            if field
                .min
                .as_ref()
                .zip(field.max.as_ref())
                .is_some_and(|(a, b)| a.as_f64() > b.as_f64())
                || field
                    .step
                    .as_ref()
                    .is_some_and(|n| n.as_f64().is_none_or(|v| !v.is_finite() || v <= 0.0))
            {
                return Err(format!("state.{key}: invalid numeric bounds"));
            }
            if field.kind == UiStateKind::Number && (field.min.is_none() || field.max.is_none()) {
                return Err(format!("state.{key}: numeric controls require min and max"));
            }
            let mut options = BTreeSet::new();
            for o in &field.options {
                if !ui_identifier(&o.value) || !options.insert(&o.value) {
                    return Err(format!("state.{key}: invalid options"));
                }
                super::text(&o.label, "option label", 160, true)?;
            }
        }
        for key in self.data.keys() {
            if !ui_identifier(key) {
                return Err(format!("data.{key}: invalid key"));
            }
        }
        for (key, value) in &self.computed {
            if !ui_identifier(key) {
                return Err(format!("computed.{key}: invalid key"));
            }
            expression::validate(value, 0).map_err(|e| format!("computed.{key}: {e}"))?;
        }
        fn dependencies(value: &Value, result: &mut BTreeSet<String>) {
            match value {
                Value::Object(map) if map.contains_key("$literal") => {}
                Value::Object(map) => {
                    if let Some(path) = map
                        .get("$get")
                        .and_then(Value::as_str)
                        .and_then(|path| path.strip_prefix("computed."))
                        && let Some(key) = path.split('.').next()
                    {
                        result.insert(key.into());
                    }
                    for value in map.values() {
                        dependencies(value, result);
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        dependencies(value, result);
                    }
                }
                _ => {}
            }
        }
        fn visit(
            key: &str,
            program: &UiProgram,
            active: &mut BTreeSet<String>,
            done: &mut BTreeSet<String>,
        ) -> Result<(), String> {
            if done.contains(key) {
                return Ok(());
            }
            if !active.insert(key.into()) {
                return Err(format!("cyclic computed dependency {key}"));
            }
            let value = program
                .computed
                .get(key)
                .ok_or_else(|| format!("unknown computed dependency {key}"))?;
            let mut links = BTreeSet::new();
            dependencies(value, &mut links);
            for link in links {
                visit(&link, program, active, done)?;
            }
            active.remove(key);
            done.insert(key.into());
            Ok(())
        }
        let mut done = BTreeSet::new();
        for key in self.computed.keys() {
            visit(key, self, &mut BTreeSet::new(), &mut done)?;
        }
        for (key, actions) in &self.actions {
            if !ui_identifier(key) || actions.is_empty() || actions.len() > 16 {
                return Err(format!("action.{key}: invalid action list"));
            }
            for (i, action) in actions.iter().enumerate() {
                let path = format!("actions.{key}[{i}]");
                match action {
                    UiAction::Set { key, value } => {
                        if !self.state.contains_key(key) {
                            return Err(format!("{path}.key: unknown state {key}"));
                        }
                        expression::validate(value, 0).map_err(|e| format!("{path}.value: {e}"))?;
                    }
                    UiAction::Toggle { key } => {
                        if self
                            .state
                            .get(key)
                            .is_none_or(|s| s.kind != UiStateKind::Boolean)
                        {
                            return Err(format!("{path}.key: toggle requires boolean state {key}"));
                        }
                    }
                    UiAction::Copy { text } => {
                        expression::validate(text, 0).map_err(|e| format!("{path}.text: {e}"))?
                    }
                    UiAction::Reset => {}
                }
            }
        }
        let nodes = self
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n))
            .collect::<BTreeMap<_, _>>();
        fn walk<'a>(
            id: &'a str,
            nodes: &BTreeMap<&'a str, &'a UiNode>,
            visited: &mut BTreeSet<&'a str>,
            depth: usize,
        ) -> Result<(), String> {
            if depth > 12 || !visited.insert(id) {
                return Err(format!(
                    "node {id}: cyclic/shared subtree or depth exceeds 12"
                ));
            }
            let node = nodes
                .get(id)
                .ok_or_else(|| format!("node {id}: missing child"))?;
            for child in &node.children {
                walk(child, nodes, visited, depth + 1)?;
            }
            Ok(())
        }
        let mut visited = BTreeSet::new();
        walk(&self.root, &nodes, &mut visited, 0)?;
        if visited.len() != nodes.len() {
            return Err("program contains unreachable nodes".into());
        }
        // Validate all computed dependencies, including initially hidden branches.
        let state = self.reconcile_state(&UiState::new());
        let mut evaluator = expression::Evaluator::new(self, &state);
        for key in self.computed.keys() {
            evaluator
                .computed(key)
                .map_err(|e| format!("computed.{key}: {e}"))?;
        }
        self.render(&state)?;
        Ok(())
    }
    pub fn render(&self, state: &UiState) -> Result<UiView, String> {
        let state = self.reconcile_state(state);
        let mut evaluator = expression::Evaluator::new(self, &state);
        let nodes = self
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n))
            .collect::<BTreeMap<_, _>>();
        fn resolve(
            id: &str,
            nodes: &BTreeMap<&str, &UiNode>,
            e: &mut expression::Evaluator<'_>,
            state: &UiState,
            depth: usize,
        ) -> Result<Option<UiResolvedNode>, String> {
            if depth > 12 {
                return Err("render depth exceeded".into());
            }
            let node = nodes.get(id).ok_or_else(|| format!("missing node {id}"))?;
            let index = e.program.nodes.iter().position(|n| n.id == id).unwrap_or(0);
            if let Some(visible) = &node.visible {
                let value = e
                    .eval(visible, &Value::Null)
                    .map_err(|error| format!("nodes[{index}].visible: {error}"))?;
                if !value
                    .as_bool()
                    .ok_or_else(|| format!("nodes[{index}].visible: must evaluate to a boolean"))?
                {
                    return Ok(None);
                }
            }
            let mut props = BTreeMap::new();
            for (k, v) in &node.props {
                props.insert(
                    k.clone(),
                    e.eval(v, &Value::Null)
                        .map_err(|error| format!("nodes[{index}].props.{k}: {error}"))?,
                );
            }
            presentation::validate_props(node.kind, &props).map_err(|err| {
                let index = e.program.nodes.iter().position(|n| n.id == id).unwrap_or(0);
                format!("nodes[{index}].props: {err}")
            })?;
            let mut children = Vec::new();
            for child in &node.children {
                if let Some(view) = resolve(child, nodes, e, state, depth + 1)? {
                    children.push(view);
                }
            }
            Ok(Some(UiResolvedNode {
                id: node.id.clone(),
                kind: node.kind,
                props,
                children,
                bind: node.bind.clone(),
                value: node
                    .bind
                    .as_ref()
                    .and_then(|k| state.get(k))
                    .cloned()
                    .unwrap_or(Value::Null),
                field: node
                    .bind
                    .as_ref()
                    .and_then(|k| e.program.state.get(k))
                    .cloned(),
                action: node.action.clone(),
            }))
        }
        let root = resolve(&self.root, &nodes, &mut evaluator, &state, 0)?
            .ok_or("root must be visible")?;
        let computed = evaluator.cache.clone();
        Ok(UiView {
            root,
            state,
            computed,
        })
    }
    pub fn input(
        &self,
        state: &UiState,
        node_id: &str,
        value: Value,
    ) -> Result<UiActionResult, String> {
        let view = self.render(state)?;
        let node = view.root.find(node_id).ok_or("control is not visible")?;
        if node.props.get("disabled").and_then(Value::as_bool) == Some(true) {
            return Err("control is disabled".into());
        }
        let key = node.bind.as_ref().ok_or("node is not an input")?;
        let field = self.state.get(key).ok_or("unknown state")?;
        if !field.accepts(&value) {
            return Err(format!("invalid value for state {key}"));
        }
        let mut next = view.state;
        next.insert(key.clone(), value);
        self.render(&next)?;
        Ok(UiActionResult {
            state: next,
            copy: None,
        })
    }
    pub fn activate(&self, state: &UiState, node_id: &str) -> Result<UiActionResult, String> {
        let view = self.render(state)?;
        let node = view.root.find(node_id).ok_or("button is not visible")?;
        if node.kind != UiKind::Button
            || node.props.get("disabled").and_then(Value::as_bool) == Some(true)
        {
            return Err("button is disabled".into());
        }
        let action = node.action.as_ref().ok_or("missing action")?;
        let actions = self.actions.get(action).ok_or("unknown action")?;
        let mut next = view.state;
        let mut copy = None;
        for action in actions {
            match action {
                UiAction::Set { key, value } => {
                    let evaluated =
                        expression::Evaluator::new(self, &next).eval(value, &Value::Null)?;
                    if !self.state.get(key).is_some_and(|s| s.accepts(&evaluated)) {
                        return Err(format!("action produced invalid state {key}"));
                    }
                    next.insert(key.clone(), evaluated);
                }
                UiAction::Toggle { key } => {
                    let value = next
                        .get(key)
                        .and_then(Value::as_bool)
                        .ok_or("toggle expects boolean")?;
                    next.insert(key.clone(), json!(!value));
                }
                UiAction::Reset => next = self.reconcile_state(&UiState::new()),
                UiAction::Copy { text } => {
                    let value = expression::Evaluator::new(self, &next).eval(text, &Value::Null)?;
                    let text = value.as_str().ok_or("copy expects text")?;
                    if text.len() > 32 * 1024 {
                        return Err("copy exceeds 32 KiB".into());
                    }
                    copy = Some(text.to_owned());
                }
            }
        }
        self.render(&next)?;
        Ok(UiActionResult { state: next, copy })
    }
}
impl UiResolvedNode {
    pub fn find(&self, id: &str) -> Option<&Self> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|n| n.find(id))
    }
    pub fn controls(&self) -> Vec<&Self> {
        let mut out = Vec::new();
        if self.kind.is_control() {
            out.push(self);
        }
        for child in &self.children {
            out.extend(child.controls());
        }
        out
    }
}
