//! Runtime-neutral permission types shared across layers.

mod network;
#[cfg(test)]
mod network_identity_tests;
#[cfg(all(test, unix))]
mod path_alias_tests;
pub(crate) mod path_identity;
pub(crate) mod protected_paths;
mod sandbox;
pub use network::NetworkPermissionPolicy;
pub use sandbox::{
    SandboxFilesystemPolicy, SandboxFilesystemRestrictions, SandboxPathAccess, SandboxPathPattern,
    SandboxPathPolicy, SandboxPathRule, SandboxPermissionSnapshot, SandboxPersistedRule,
    SandboxPolicyError,
};

use agena_domain::{
    AccessKind, AccessSelector, NetworkTarget, PermissionAction, PermissionDecision,
    PermissionMode, decide_from_mode,
};
use agena_plugin_sdk::{PluginKey, ToolTag};
use path_clean::PathClean;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::command_class::{self, CommandClass};

/// The built-in mode of each shell command class, applied when the
/// configuration carries no `tools.rules."shell.*"` entry for that class.
/// These reproduce the rules the automatic-approval fast path and the shell
/// heuristics applied before they became configuration.
pub(crate) const DEFAULT_NO_OP_COMMANDS: PermissionMode = PermissionMode::Allow;
pub(crate) const DEFAULT_ROUTINE_COMMANDS: PermissionMode = PermissionMode::Allow;
pub(crate) const DEFAULT_DANGEROUS_COMMANDS: PermissionMode = PermissionMode::Deny;

/// The built-in mode of the read-only tool class.
pub(crate) const DEFAULT_READ_ONLY_TOOLS: PermissionMode = PermissionMode::Allow;

pub(crate) fn built_in_command_class_mode(class: CommandClass) -> PermissionMode {
    match class {
        CommandClass::NoOp => DEFAULT_NO_OP_COMMANDS,
        CommandClass::Routine => DEFAULT_ROUTINE_COMMANDS,
        CommandClass::Dangerous => DEFAULT_DANGEROUS_COMMANDS,
    }
}

pub(crate) fn command_class_label(class: CommandClass) -> &'static str {
    match class {
        CommandClass::NoOp => "no-op",
        CommandClass::Routine => "routine",
        CommandClass::Dangerous => "dangerous shell",
    }
}

#[derive(Debug, Clone)]
/// Permission policy for tool execution.
pub struct ToolPermissionPolicy {
    pub(crate) disabled_plugins: HashSet<PluginKey>,
    pub(crate) default_mode: PermissionMode,
    pub(crate) tool_modes: HashMap<String, PermissionMode>,
    pub(crate) bash_pattern_rules: Vec<BashPatternRule>,
    pub(crate) bash_deny_rules: Vec<BashPatternRule>,
    pub(crate) bash_overlay_rules: Vec<BashPatternRule>,
    /// The read-only tool class, written as `tools.rules."*".read-only`.
    /// `None` keeps the built-in class default.
    pub(crate) read_only_mode: Option<PermissionMode>,
}

#[derive(Debug, Clone)]
/// A bash command pattern rule with a permission mode.
pub struct BashPatternRule {
    matcher: CommandPatternMatcher,
    pattern: String,
    mode: PermissionMode,
}

impl BashPatternRule {
    pub fn new_wildcard(pattern: impl Into<String>, mode: PermissionMode) -> Self {
        let pattern = pattern.into();
        Self {
            matcher: CommandPatternMatcher::Wildcard(WildcardPattern::new(&pattern)),
            pattern,
            mode,
        }
    }

    /// A rule matching a whole command class rather than a command text. The
    /// keyword is kept as the rule's `pattern` so a decision reads the same
    /// way a pattern rule's does.
    pub fn new_class(
        class: CommandClass,
        keyword: impl Into<String>,
        mode: PermissionMode,
    ) -> Self {
        Self {
            matcher: CommandPatternMatcher::Class(class),
            pattern: keyword.into(),
            mode,
        }
    }

    fn matches(&self, input: &str) -> bool {
        self.matcher.matches(input)
    }
}

#[derive(Debug, Clone)]
enum CommandPatternMatcher {
    Wildcard(WildcardPattern),
    Class(CommandClass),
}

impl CommandPatternMatcher {
    fn matches(&self, input: &str) -> bool {
        match self {
            Self::Wildcard(pattern) => pattern.matches(input),
            Self::Class(class) => command_class::command_classes(input) == Some(*class),
        }
    }
}

#[derive(Debug, Clone)]
struct WildcardPattern {
    pattern: String,
    optional_prefix: Option<String>,
}

impl WildcardPattern {
    fn new(pattern: impl Into<String>) -> Self {
        let mut pattern = pattern.into().replace('\\', "/");
        if cfg!(windows) {
            pattern.make_ascii_lowercase();
        }
        let optional_prefix = pattern.strip_suffix(" *").map(ToOwned::to_owned);
        Self {
            pattern,
            optional_prefix,
        }
    }

    fn matches_path(&self, input: &str) -> bool {
        if !self.pattern.contains(['*', '?']) {
            return path_identity::equal(Path::new(&self.pattern), Path::new(input));
        }
        let prefix = self.pattern.trim_end_matches('*');
        if prefix.len() != self.pattern.len() && !prefix.contains(['*', '?']) {
            if prefix.is_empty() {
                return true;
            }
            if prefix.ends_with('/') {
                let root = Path::new(prefix);
                let target = Path::new(input);
                return path_identity::within(target, root) && !path_identity::equal(target, root);
            }
        }
        // Legacy in-memory glob rules retain their previous matching behavior.
        // New resource settings compile only literals and complete subtrees.
        self.matches(input)
    }

    fn matches(&self, input: &str) -> bool {
        let mut normalized = input.replace('\\', "/");
        if cfg!(windows) {
            normalized.make_ascii_lowercase();
        }
        self.optional_prefix
            .as_ref()
            .is_some_and(|prefix| wildcard_match(prefix, &normalized))
            || wildcard_match(&self.pattern, &normalized)
    }
}

fn wildcard_match(pattern: &str, input: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let input = input.chars().collect::<Vec<_>>();
    let mut pattern_index = 0usize;
    let mut input_index = 0usize;
    let mut star_index = None;
    let mut star_input_index = 0usize;

    while input_index < input.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == '?' || pattern[pattern_index] == input[input_index])
        {
            pattern_index += 1;
            input_index += 1;
            continue;
        }

        if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            star_index = Some(pattern_index);
            pattern_index += 1;
            star_input_index = input_index;
            continue;
        }

        if let Some(saved_star_index) = star_index {
            pattern_index = saved_star_index + 1;
            star_input_index += 1;
            input_index = star_input_index;
            continue;
        }

        return false;
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
}

pub fn bash_rule_qualifier(command: &str, rules: &[BashPatternRule]) -> Option<String> {
    let normalized = command.trim();
    if normalized.is_empty() {
        return None;
    }
    rules
        .iter()
        .find(|rule| rule.matches(normalized))
        .map(|rule| rule.pattern.clone())
}

pub fn bash_permission_qualifier(
    command: &str,
    policy: Option<&ToolPermissionPolicy>,
) -> Option<String> {
    let normalized = command.trim();
    if normalized.is_empty() {
        return None;
    }
    policy
        .and_then(|policy| {
            bash_rule_qualifier(normalized, policy.bash_deny_rules())
                .or_else(|| bash_rule_qualifier_reverse(normalized, policy.bash_overlay_rules()))
                .or_else(|| bash_rule_qualifier(normalized, policy.bash_pattern_rules()))
        })
        .or_else(|| Some(normalized.to_string()))
}

fn bash_rule_qualifier_reverse(command: &str, rules: &[BashPatternRule]) -> Option<String> {
    let normalized = command.trim();
    if normalized.is_empty() {
        return None;
    }
    rules
        .iter()
        .rev()
        .find(|rule| rule.matches(normalized))
        .map(|rule| rule.pattern.clone())
}

fn is_terminal_write(names: &[&str]) -> bool {
    names.iter().any(|name| {
        matches!(
            *name,
            "shell.write" | "agena.shell.write" | "agena_shell_write"
        )
    })
}

pub fn tool_action(
    tool_name: &str,
    command: Option<&str>,
    tags: &[ToolTag],
    policy: Option<&ToolPermissionPolicy>,
) -> PermissionAction {
    let qualifier = if ToolTag::is_shell(tags) && !is_terminal_write(&[tool_name]) {
        command.and_then(|command| bash_permission_qualifier(command, policy))
    } else {
        None
    };
    PermissionAction::Tool {
        tool_name: tool_name.to_string(),
        qualifier,
    }
}

impl ToolPermissionPolicy {
    pub fn new(default_mode: PermissionMode) -> Self {
        Self {
            disabled_plugins: HashSet::new(),
            default_mode,
            tool_modes: HashMap::new(),
            bash_pattern_rules: Vec::new(),
            bash_deny_rules: Vec::new(),
            bash_overlay_rules: Vec::new(),
            read_only_mode: None,
        }
    }

    pub fn allow_all() -> Self {
        Self::new(PermissionMode::Allow)
    }

    /// Append an overlay bash command pattern rule using shell-style wildcard
    /// semantics. These rules are evaluated after unconditional deny
    /// patterns but before the base bash pattern rules, and the last matching
    /// overlay rule wins.
    pub fn add_bash_overlay_rule(&mut self, pattern: impl Into<String>, mode: PermissionMode) {
        self.bash_overlay_rules
            .push(BashPatternRule::new_wildcard(pattern, mode));
    }

    /// Append a command-class rule. It is stored among the overlay rules so it
    /// keeps the ranking a `tools.rules."shell.*"` entry has always had — after
    /// the unconditional deny patterns and the user's command patterns, and
    /// before the tool's own default.
    pub fn add_bash_class_rule(
        &mut self,
        class: CommandClass,
        keyword: impl Into<String>,
        mode: PermissionMode,
    ) {
        self.bash_overlay_rules
            .push(BashPatternRule::new_class(class, keyword, mode));
    }

    /// Install the read-only tool class, written as `tools.rules."*".read-only`.
    pub fn set_read_only_mode(&mut self, mode: Option<PermissionMode>) {
        self.read_only_mode = mode;
    }

    pub fn bash_pattern_rules(&self) -> &[BashPatternRule] {
        &self.bash_pattern_rules
    }

    pub fn bash_deny_rules(&self) -> &[BashPatternRule] {
        &self.bash_deny_rules
    }

    pub fn bash_overlay_rules(&self) -> &[BashPatternRule] {
        &self.bash_overlay_rules
    }

    pub fn check_tool_with_names(
        &self,
        names: &[&str],
        command: Option<&str>,
        tags: &[ToolTag],
    ) -> PermissionDecision {
        // Input to a persistent CLI is not an independent shell command: it
        // may complete an earlier input or execute inside a REPL. Keep explicit
        // tool restrictions and shared command restrictions, while an allow
        // or auto rule for a command cannot approve arbitrary terminal input.
        if is_terminal_write(names) {
            let mut decision = self.check_tool_mode_with_names(names);
            if matches!(decision, PermissionDecision::Deny { .. }) {
                return decision;
            }
            if let Some(command) = command {
                for rule_decision in [
                    self.evaluate_bash_deny(command),
                    self.evaluate_bash_overlay_pattern(command),
                    self.evaluate_bash_pattern(command),
                    self.evaluate_command_class(command),
                ]
                .into_iter()
                .flatten()
                {
                    match rule_decision {
                        PermissionDecision::Deny { .. } => return rule_decision,
                        PermissionDecision::Ask { .. } => decision = rule_decision,
                        _ => {}
                    }
                }
            }
            return decision;
        }
        let named_mode = self.tool_name_mode(names);
        if let Some((matched_name, PermissionMode::Deny)) = named_mode {
            return self.decision_for_mode(matched_name, PermissionMode::Deny);
        }
        if ToolTag::is_shell(tags)
            && let Some(command) = command
        {
            if let Some(decision) = self.evaluate_bash_deny(command) {
                return decision;
            }
            // An explicit command restriction cannot be bypassed by a broad
            // tool-name Allow. Class defaults remain below named decisions.
            if let Some(decision) = self.evaluate_bash_overlay_pattern(command) {
                return decision;
            }
            if let Some((matched_name, mode)) = named_mode {
                return self.decision_for_mode(matched_name, mode);
            }
            if let Some(decision) = self.evaluate_bash_pattern(command) {
                return decision;
            }
            // The command classes are defaults: a command the user named under
            // `tools.rules` was answered above, and a command no rule names
            // lands on its class default.
            if let Some(decision) = self.evaluate_command_class(command) {
                return decision;
            }
        }
        if let Some((matched_name, mode)) = named_mode {
            return self.decision_for_mode(matched_name, mode);
        }
        // The read-only class default. Like the interaction default it is
        // ranked above `tools.default`, and below a rule naming the tool.
        if let Some(decision) = self.evaluate_read_only_class(names, tags) {
            return decision;
        }
        let name = names.first().copied().unwrap_or("tool");
        self.decision_for_mode(name, self.default_mode)
    }

    /// Permission aliases describe the same tool. A restriction follows the
    /// tool even if an invocation or another setting uses a different alias.
    fn tool_name_mode<'a>(&self, names: &[&'a str]) -> Option<(&'a str, PermissionMode)> {
        names
            .iter()
            .filter_map(|name| self.tool_modes.get(*name).map(|mode| (*name, *mode)))
            .max_by_key(|(_, mode)| match mode {
                PermissionMode::Allow => 0,
                PermissionMode::Auto => 1,
                PermissionMode::Ask => 2,
                PermissionMode::Deny => 3,
            })
    }

    pub(crate) fn tool_is_discoverable(&self, names: &[&str], tags: &[ToolTag]) -> bool {
        if !matches!(
            self.check_tool_with_names(names, None, tags),
            PermissionDecision::Deny { .. }
        ) {
            return true;
        }
        // A shell command can be authorized by its own rule when only the
        // default tool mode denies. The real command is checked on invocation.
        if !ToolTag::is_shell(tags)
            || is_terminal_write(names)
            || self.tool_name_mode(names).is_some()
        {
            return false;
        }
        if self.bash_deny_rules.iter().any(|rule| rule.pattern == "*") {
            return false;
        }
        for rule in self
            .bash_overlay_rules
            .iter()
            .rev()
            .filter(|rule| matches!(rule.matcher, CommandPatternMatcher::Wildcard(_)))
            .chain(&self.bash_pattern_rules)
        {
            if rule.mode != PermissionMode::Deny {
                return true;
            }
            if rule.pattern == "*" {
                return false;
            }
        }
        [
            CommandClass::NoOp,
            CommandClass::Routine,
            CommandClass::Dangerous,
        ]
        .into_iter()
        .any(|class| {
            matches!(
                self.command_class_mode(class),
                PermissionMode::Allow | PermissionMode::Ask
            )
        })
    }

    /// The default for tools whose tags put them in the read-only class, on
    /// the plugin's own word. `None` means no default applies, so the tool
    /// falls through to `tools.default`.
    ///
    /// The mode comes from `tools.rules."*".read-only` when the configuration
    /// sets one, and from the built-in `allow` otherwise. A tool the user named
    /// under `tools.names` was already answered by [`Self::tool_name_mode`],
    /// so this class default ranks below a name and above `tools.default`.
    fn evaluate_read_only_class(
        &self,
        names: &[&str],
        tags: &[ToolTag],
    ) -> Option<PermissionDecision> {
        if !ToolTag::is_read_only(tags) {
            return None;
        }
        let mode = self.read_only_mode.unwrap_or(DEFAULT_READ_ONLY_TOOLS);
        if mode == PermissionMode::Auto {
            return None;
        }
        let name = names.first().copied().unwrap_or("tool");
        Some(self.decision_for_mode(name, mode))
    }

    pub fn check_tool(
        &self,
        name: &str,
        command: Option<&str>,
        tags: &[ToolTag],
    ) -> PermissionDecision {
        self.check_tool_with_names(&[name], command, tags)
    }

    fn check_tool_mode_with_names(&self, names: &[&str]) -> PermissionDecision {
        // A precise tool-name rule wins; otherwise the default applies. The
        // default for ordinary execution tools is Allow: most tools are safe
        // because their effects are already constrained by the path, network,
        // and shell-command policies. Ask/Deny remain for the cases that need
        // them (users who opt into stricter tool gating). The declared tags
        // are never used as a proxy for the default — only configured rules
        // and `tools.default` decide.
        if let Some((matched_name, mode)) = self.tool_name_mode(names) {
            return self.decision_for_mode(matched_name, mode);
        }
        let name = names.first().copied().unwrap_or("tool");
        self.decision_for_mode(name, self.default_mode)
    }

    fn decision_for_mode(&self, name: &str, mode: PermissionMode) -> PermissionDecision {
        match mode {
            PermissionMode::Allow => PermissionDecision::Allow,
            PermissionMode::Auto => PermissionDecision::Auto {
                reason: format!("tool '{name}' is eligible for automatic approval"),
            },
            PermissionMode::Ask => PermissionDecision::Ask {
                reason: format!("tool '{name}' requires confirmation by policy"),
            },
            PermissionMode::Deny => PermissionDecision::Deny {
                reason: format!("tool '{name}' denied by policy"),
            },
        }
    }

    /// Answer a shell command by its built-in class default.
    ///
    /// The class rules live among the overlay rules (they are
    /// `tools.rules."shell.*"` entries like any other), so a command no
    /// pattern names falls to its class's rule — the built-in
    /// `allow` / `allow` / `deny` when the configuration carries none.
    /// A class left at `Auto` is not decided statically and reaches the
    /// approval model.
    fn evaluate_command_class(&self, command: &str) -> Option<PermissionDecision> {
        let normalized = command.trim();
        if normalized.is_empty() {
            return None;
        }
        let class = crate::command_class::command_classes(normalized)?;
        let mode = self.command_class_mode(class);
        if mode == PermissionMode::Auto {
            return None;
        }
        let label = command_class_label(class);
        Some(match mode {
            PermissionMode::Allow => PermissionDecision::Allow,
            PermissionMode::Ask => PermissionDecision::Ask {
                reason: format!(
                    "automatic approval class default requires confirmation for this {label} command"
                ),
            },
            PermissionMode::Deny => PermissionDecision::Deny {
                reason: format!("automatic approval class default blocked this {label} command"),
            },
            PermissionMode::Auto => unreachable!("handled above"),
        })
    }

    fn command_class_mode(&self, class: CommandClass) -> PermissionMode {
        self.bash_overlay_rules
            .iter()
            .rev()
            .find(|rule| matches!(rule.matcher, CommandPatternMatcher::Class(rule_class) if rule_class == class))
            .map_or_else(|| built_in_command_class_mode(class), |rule| rule.mode)
    }

    fn evaluate_bash_pattern(&self, command: &str) -> Option<PermissionDecision> {
        let normalized = command.trim();
        if normalized.is_empty() {
            return None;
        }
        for rule in &self.bash_pattern_rules {
            if rule.matches(normalized) {
                let decision = match rule.mode {
                    PermissionMode::Allow => PermissionDecision::Allow,
                    PermissionMode::Auto => PermissionDecision::Auto {
                        reason: format!(
                            "bash command matches `{}` and is eligible for automatic approval",
                            rule.pattern
                        ),
                    },
                    PermissionMode::Ask => PermissionDecision::Ask {
                        reason: format!(
                            "bash command matches `{}` and requires confirmation",
                            rule.pattern
                        ),
                    },
                    PermissionMode::Deny => PermissionDecision::Deny {
                        reason: format!(
                            "bash command matches `{}` and is denied by policy",
                            rule.pattern
                        ),
                    },
                };
                return Some(decision);
            }
        }
        None
    }

    fn evaluate_bash_overlay_pattern(&self, command: &str) -> Option<PermissionDecision> {
        let normalized = command.trim();
        if normalized.is_empty() {
            return None;
        }
        for rule in self.bash_overlay_rules.iter().rev() {
            if matches!(rule.matcher, CommandPatternMatcher::Wildcard(_))
                && rule.matches(normalized)
            {
                let decision = match rule.mode {
                    PermissionMode::Allow => PermissionDecision::Allow,
                    PermissionMode::Auto => PermissionDecision::Auto {
                        reason: format!(
                            "bash command matches `{}` and is eligible for automatic approval",
                            rule.pattern
                        ),
                    },
                    PermissionMode::Ask => PermissionDecision::Ask {
                        reason: format!(
                            "bash command matches `{}` and requires confirmation",
                            rule.pattern
                        ),
                    },
                    PermissionMode::Deny => PermissionDecision::Deny {
                        reason: format!(
                            "bash command matches `{}` and is denied by policy",
                            rule.pattern
                        ),
                    },
                };
                return Some(decision);
            }
        }
        None
    }

    fn evaluate_bash_deny(&self, command: &str) -> Option<PermissionDecision> {
        let normalized = command.trim();
        if normalized.is_empty() {
            return None;
        }
        for rule in &self.bash_deny_rules {
            if rule.matches(normalized) {
                return Some(PermissionDecision::Deny {
                    reason: format!(
                        "bash command matches deny pattern `{}` and is unconditionally blocked",
                        rule.pattern
                    ),
                });
            }
        }
        None
    }
}

pub fn combine_permission_modes(left: PermissionMode, right: PermissionMode) -> PermissionMode {
    match (left, right) {
        (PermissionMode::Deny, _) | (_, PermissionMode::Deny) => PermissionMode::Deny,
        (PermissionMode::Ask, _) | (_, PermissionMode::Ask) => PermissionMode::Ask,
        (PermissionMode::Auto, _) | (_, PermissionMode::Auto) => PermissionMode::Auto,
        (PermissionMode::Allow, PermissionMode::Allow) => PermissionMode::Allow,
    }
}

#[derive(Debug, Error)]
/// Error building a permission policy from configuration.
pub enum PermissionConfigError {
    #[error("invalid permission path rule `{pattern}`: {reason}")]
    InvalidPathRule { pattern: String, reason: String },
    #[error("unknown permission path marker `{alias}` in pattern `{pattern}`")]
    UnknownPathAlias { pattern: String, alias: String },
    #[error("permission path marker `{alias}` cannot be resolved for pattern `{pattern}`")]
    UnresolvedPathAlias { pattern: String, alias: String },
    #[error("invalid permission network rule `{pattern}`: {reason}")]
    InvalidNetworkRule { pattern: String, reason: String },
    #[error("invalid permission.plugins key {plugin:?}: use an exact namespace.plugin ID")]
    InvalidPluginId { plugin: String },
    #[error("invalid permission.tools.rules[{tool:?}][{pattern:?}]: {reason}")]
    InvalidToolRule {
        tool: String,
        pattern: String,
        reason: String,
    },
    #[error("{field} cannot be enforced by the current MXC sandbox: {reason}")]
    UnsupportedSandbox { field: String, reason: String },
}

#[derive(Debug, Clone)]
/// Permission policy for workspace and external paths.
pub struct PermissionPolicy {
    pub(crate) workspace_read_default: PermissionMode,
    pub(crate) workspace_write_default: PermissionMode,
    pub(crate) external_read_default: PermissionMode,
    pub(crate) external_write_default: PermissionMode,
    pub(crate) rules: Vec<PermissionRule>,
}

impl PermissionPolicy {
    pub fn new(workspace_read: PermissionMode, workspace_write: PermissionMode) -> Self {
        Self {
            workspace_read_default: workspace_read,
            workspace_write_default: workspace_write,
            external_read_default: workspace_read,
            external_write_default: workspace_write,
            rules: Vec::new(),
        }
    }

    pub fn allow_all() -> Self {
        Self::new(PermissionMode::Allow, PermissionMode::Allow)
    }

    pub fn add_path_pattern_rule(
        &mut self,
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<(), PermissionConfigError> {
        self.rules
            .push(PermissionRule::path_pattern(selector, mode, pattern)?);
        Ok(())
    }

    pub fn check_access(
        &self,
        access: AccessKind,
        workspace_root: &Path,
        target_path: &Path,
    ) -> PermissionDecision {
        let context = MatchContext::new(workspace_root, target_path);
        self.check_access_with_context(access, &context)
    }

    /// A trusted setting for a specific Git resource can override its default
    /// protection. A broad workspace or directory grant cannot override it.
    fn has_explicit_write_grant(&self, workspace: &Path, path: &Path) -> bool {
        let context = MatchContext::new(workspace, path);
        self.rules.iter().any(|rule| {
            let RuleMatcher::PathPattern(pattern) = &rule.matcher;
            rule.matches_selector(AccessKind::Write)
                && rule.mode == PermissionMode::Allow
                && pattern.matches_literal(&context)
        })
    }

    fn check_access_with_context(
        &self,
        access: AccessKind,
        context: &MatchContext,
    ) -> PermissionDecision {
        for rule in self.rules.iter().rev() {
            if !rule.matches_selector(access) {
                continue;
            }
            if rule.matcher.matches(context) {
                return decide_from_mode(rule.mode, &rule.description);
            }
        }

        match (access, context.in_workspace) {
            (AccessKind::Read, true) => decide_from_mode(
                self.workspace_read_default,
                "matched workspace default read permission",
            ),
            (AccessKind::Write, true) => decide_from_mode(
                self.workspace_write_default,
                "matched workspace default write permission",
            ),
            (AccessKind::Read, false) => decide_from_mode(
                self.external_read_default,
                "matched external default read permission",
            ),
            (AccessKind::Write, false) => decide_from_mode(
                self.external_write_default,
                "matched external default write permission",
            ),
        }
    }
}

#[derive(Debug, Clone)]
/// A single permission rule.
pub struct PermissionRule {
    selector: AccessSelector,
    mode: PermissionMode,
    matcher: RuleMatcher,
    description: String,
}

impl PermissionRule {
    pub fn path_pattern(
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        let pattern = pattern.into();
        Ok(Self {
            selector,
            mode,
            matcher: RuleMatcher::PathPattern(PathPattern::new(pattern.as_str())?),
            description: format!("matched path pattern: {pattern}"),
        })
    }

    fn matches_selector(&self, access: AccessKind) -> bool {
        matches!(
            (self.selector, access),
            (AccessSelector::Any, _)
                | (AccessSelector::Read, AccessKind::Read)
                | (AccessSelector::Write, AccessKind::Write)
        )
    }
}

#[derive(Debug, Clone)]
enum RuleMatcher {
    PathPattern(PathPattern),
}

impl RuleMatcher {
    fn matches(&self, ctx: &MatchContext) -> bool {
        match self {
            Self::PathPattern(pattern) => pattern.matches(ctx),
        }
    }
}

#[derive(Debug, Clone)]
enum PathPattern {
    Workspace(WildcardPattern),
    ManagedWorkspace(WildcardPattern),
    Absolute(WildcardPattern),
    /// One alias that resolves to several roots. The pattern matches when any
    /// of them matches, which is what `<tmp>` needs: the private per-user temp
    /// directory and the classic `/tmp` location are different paths.
    AbsoluteAny(Vec<WildcardPattern>),
}

impl PathPattern {
    fn matches_literal(&self, ctx: &MatchContext) -> bool {
        let matches = |pattern: &WildcardPattern, value: &str| {
            !pattern.pattern.contains(['*', '?']) && pattern.matches_path(value)
        };
        match self {
            Self::Workspace(pattern) => ctx
                .workspace_relative_norm
                .as_deref()
                .is_some_and(|path| matches(pattern, path)),
            Self::ManagedWorkspace(pattern) => ctx
                .managed_relative_norm
                .as_deref()
                .is_some_and(|path| matches(pattern, path)),
            Self::Absolute(pattern) => matches(pattern, &ctx.absolute_norm),
            Self::AbsoluteAny(patterns) => patterns
                .iter()
                .any(|pattern| matches(pattern, &ctx.absolute_norm)),
        }
    }

    fn new(pattern: &str) -> Result<Self, PermissionConfigError> {
        let normalized = pattern.trim().replace('\\', "/");
        if normalized.is_empty() {
            return Ok(Self::Workspace(WildcardPattern::new("")));
        }
        if let Some(rest) = strip_path_alias(&normalized, "cwd")
            .or_else(|| strip_path_alias(&normalized, "workspace"))
        {
            return Ok(Self::Workspace(WildcardPattern::new(
                workspace_alias_pattern(rest),
            )));
        }
        if let Some(rest) = strip_path_alias(&normalized, "state") {
            return Ok(Self::ManagedWorkspace(WildcardPattern::new(
                workspace_alias_pattern(rest),
            )));
        }
        if let Some(rest) = strip_path_alias(&normalized, "home") {
            return absolute_alias_patterns(
                &normalized,
                "home",
                home_dir().into_iter().collect(),
                rest,
            );
        }
        if let Some(rest) = strip_path_alias(&normalized, "tmp") {
            return absolute_alias_patterns(&normalized, "tmp", temp_roots(), rest);
        }
        if let Some(alias) = unknown_angle_alias(&normalized) {
            return Err(PermissionConfigError::UnknownPathAlias {
                pattern: normalized,
                alias,
            });
        }
        if Path::new(&normalized).is_absolute() {
            return Ok(Self::Absolute(WildcardPattern::new(normalized)));
        }
        Ok(Self::Workspace(WildcardPattern::new(normalized)))
    }

    fn matches(&self, ctx: &MatchContext) -> bool {
        match self {
            Self::Workspace(pattern) => ctx
                .workspace_relative_norm
                .as_deref()
                .is_some_and(|relative| pattern.matches_path(relative)),
            Self::ManagedWorkspace(pattern) => ctx
                .managed_relative_norm
                .as_deref()
                .is_some_and(|relative| pattern.matches_path(relative)),
            Self::Absolute(pattern) => pattern.matches_path(&ctx.absolute_norm),
            Self::AbsoluteAny(patterns) => patterns
                .iter()
                .any(|pattern| pattern.matches_path(&ctx.absolute_norm)),
        }
    }
}

fn strip_path_alias<'a>(pattern: &'a str, alias: &str) -> Option<&'a str> {
    pattern.strip_prefix(format!("<{alias}>").as_str())
}

fn workspace_alias_pattern(rest: &str) -> String {
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        ".".to_string()
    } else if rest == "**" {
        "*".to_string()
    } else {
        rest.to_string()
    }
}

/// The roots a path alias expands to.
///
/// `<tmp>` covers every location a user and the runtime's own tooling call
/// "temp": the OS temp directory (`std::env::temp_dir()` — on macOS the private
/// per-user `$TMPDIR` under `/var/folders`) plus the classic `/tmp`, which macOS
/// resolves to `/private/tmp`. Covering only the OS directory left a path that a
/// user or a tool spelled `/tmp/...` to the external default, so writing a
/// scratch file there asked for permission even though the temp class is
/// allowed outright.
pub(crate) fn temp_roots() -> Vec<PathBuf> {
    let roots = vec![std::env::temp_dir()];
    #[cfg(unix)]
    let roots = {
        let mut roots = roots;
        roots.extend([PathBuf::from("/tmp"), PathBuf::from("/private/tmp")]);
        roots
    };
    roots
}

fn absolute_alias_patterns(
    pattern: &str,
    alias: &str,
    roots: Vec<PathBuf>,
    rest: &str,
) -> Result<PathPattern, PermissionConfigError> {
    let rest = rest.trim_start_matches('/');
    let mut expansions: Vec<String> = Vec::new();
    for root in roots {
        let canonical = crate::authorization::canonical_resource_path(&root).map_err(|reason| {
            PermissionConfigError::InvalidPathRule {
                pattern: pattern.to_owned(),
                reason,
            }
        })?;
        // Capture both the platform spelling and the real identity. Tool
        // effects use canonical paths; macOS /tmp and relocated HOME roots
        // must keep the same policy after that normalization.
        for root in [&root, &canonical] {
            let mut normalized = normalize_path_string(root);
            if normalized.is_empty() {
                continue;
            }
            if !rest.is_empty() {
                normalized.push('/');
                normalized.push_str(rest);
            }
            if !expansions.contains(&normalized) {
                expansions.push(normalized);
            }
        }
    }
    if expansions.is_empty() {
        return Err(PermissionConfigError::UnresolvedPathAlias {
            pattern: pattern.to_string(),
            alias: alias.to_string(),
        });
    }
    let mut patterns = expansions
        .into_iter()
        .map(WildcardPattern::new)
        .collect::<Vec<_>>();
    if patterns.len() == 1 {
        return Ok(PathPattern::Absolute(patterns.remove(0)));
    }
    Ok(PathPattern::AbsoluteAny(patterns))
}

fn unknown_angle_alias(pattern: &str) -> Option<String> {
    let rest = pattern.strip_prefix('<')?;
    let (alias, _) = rest.split_once('>')?;
    Some(alias.to_string())
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}

#[derive(Debug, Clone)]
struct MatchContext {
    absolute_norm: String,
    workspace_relative_norm: Option<String>,
    managed_relative_norm: Option<String>,
    in_workspace: bool,
}

impl MatchContext {
    fn new(workspace_root: &Path, target_path: &Path) -> Self {
        if workspace_root.as_os_str().is_empty() {
            return Self {
                absolute_norm: normalize_path_string(target_path),
                workspace_relative_norm: None,
                managed_relative_norm: None,
                in_workspace: false,
            };
        }
        let root_absolute = if workspace_root.is_absolute() {
            workspace_root.to_path_buf()
        } else {
            match std::env::current_dir() {
                Ok(directory) => directory.join(workspace_root),
                Err(error) => {
                    tracing::error!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "resolve the current directory for permission path matching",
                            &error,
                        ),
                        "permission path matching is using the unresolved workspace root"
                    );
                    workspace_root.to_path_buf()
                }
            }
        };
        let root_norm = normalize_path_string(&root_absolute);

        let absolute_target = if target_path.is_absolute() {
            target_path.to_path_buf()
        } else {
            root_absolute.join(target_path)
        };
        let absolute_norm = normalize_path_string(&absolute_target);

        let workspace_relative_norm =
            relative_path_norm(Path::new(&absolute_norm), Path::new(&root_norm));
        let in_workspace = workspace_relative_norm.is_some();

        let managed_root =
            agena_storage_layout::StorageLayout::current().workspace_dir(&root_absolute);
        let managed_norm = normalize_path_string(&managed_root);
        let managed_relative_norm =
            relative_path_norm(Path::new(&absolute_norm), Path::new(&managed_norm));
        Self {
            absolute_norm,
            managed_relative_norm,
            workspace_relative_norm,
            in_workspace,
        }
    }
}

fn relative_path_norm(path: &Path, root: &Path) -> Option<String> {
    path_identity::relative(path, root).map(|relative| {
        if relative.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            normalize_path_string(&relative)
        }
    })
}

fn normalize_path_string(path: &Path) -> String {
    let cleaned = path.clean();
    let mut out = cleaned.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        if let Some(ordinary) = out.strip_prefix("//?/UNC/") {
            out = format!("//{ordinary}");
        } else if let Some(ordinary) = out.strip_prefix("//?/") {
            out = ordinary.to_owned();
        }
        out.make_ascii_lowercase();
    }
    while out.ends_with('/')
        && out.len() > 1
        && !(cfg!(windows) && out.len() == 3 && out.as_bytes().get(1) == Some(&b':'))
    {
        out.pop();
    }
    out
}
