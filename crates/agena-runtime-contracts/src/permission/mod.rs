//! Runtime-neutral permission types shared across layers.

use agena_domain::{
    AccessKind, AccessSelector, NetworkTarget, PermissionAction, PermissionDecision,
    PermissionMode, decide_from_mode,
};
use agena_plugin_host::sdk::ToolTag;
use path_clean::PathClean;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::command_class::{self, CommandClass};

/// The built-in mode of each shell command class, applied when the
/// configuration carries no `tools.rules.<shell tool>` entry for that class.
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
    tool_name: Option<String>,
}

impl BashPatternRule {
    pub fn new_wildcard(pattern: impl Into<String>, mode: PermissionMode) -> Self {
        let pattern = pattern.into();
        Self {
            matcher: CommandPatternMatcher::Wildcard(WildcardPattern::new(&pattern)),
            pattern,
            mode,
            tool_name: None,
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
            tool_name: None,
        }
    }

    pub(crate) fn for_tool(mut self, name: &str) -> Self {
        self.tool_name = Some(
            normalized_shell_command_tool(name)
                .unwrap_or(name)
                .to_owned(),
        );
        self
    }

    fn applies_to(&self, names: &[&str]) -> bool {
        self.tool_name.as_deref().is_none_or(|scope| {
            names
                .iter()
                .any(|name| normalized_shell_command_tool(name).unwrap_or(name) == scope)
        })
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

pub(crate) fn normalized_shell_command_tool(name: &str) -> Option<&'static str> {
    match name {
        "shell.exec" | "agena.shell.exec" | "agena_shell_exec" => Some("agena.shell.exec"),
        "shell.spawn" | "agena.shell.spawn" | "agena_shell_spawn" => Some("agena.shell.spawn"),
        "shell.watch" | "agena.shell.watch" | "agena_shell_watch" => Some("agena.shell.watch"),
        "shell.open" | "agena.shell.open" | "agena_shell_open" => Some("agena.shell.open"),
        "shell.write" | "agena.shell.write" | "agena_shell_write" => Some("agena.shell.write"),
        "monitor.start" | "agena.monitor.start" | "agena_monitor_start" => {
            Some("agena.monitor.start")
        }
        _ => None,
    }
}

fn shell_permission_qualifier(
    command: &str,
    policy: Option<&ToolPermissionPolicy>,
    names: &[&str],
) -> Option<String> {
    let command = command.trim();
    if command.is_empty() {
        return None;
    }
    policy
        .and_then(|policy| {
            policy
                .bash_deny_rules
                .iter()
                .find(|rule| rule.applies_to(names) && rule.matches(command))
                .or_else(|| {
                    policy
                        .bash_overlay_rules
                        .iter()
                        .rev()
                        .find(|rule| rule.applies_to(names) && rule.matches(command))
                })
                .or_else(|| {
                    policy
                        .bash_pattern_rules
                        .iter()
                        .find(|rule| rule.applies_to(names) && rule.matches(command))
                })
                .map(|rule| rule.pattern.clone())
        })
        .or_else(|| Some(command.to_owned()))
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
        command.and_then(|command| shell_permission_qualifier(command, policy, &[tool_name]))
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
    /// keeps the ranking a `tools.rules.<tool>` entry has always had — after
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

    pub(crate) fn add_shell_tool_pattern(
        &mut self,
        tool: &str,
        pattern: &str,
        mode: PermissionMode,
    ) {
        self.bash_overlay_rules
            .push(BashPatternRule::new_wildcard(pattern, mode).for_tool(tool));
    }

    pub(crate) fn add_shell_tool_class(
        &mut self,
        tool: &str,
        class: CommandClass,
        keyword: &str,
        mode: PermissionMode,
    ) {
        self.bash_overlay_rules
            .push(BashPatternRule::new_class(class, keyword, mode).for_tool(tool));
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
        // tool restrictions and recognizable command denials, but never let a
        // shell prefix allow/auto rule approve arbitrary terminal input.
        if is_terminal_write(names) {
            let decision = self.check_tool_mode_with_names(names);
            if matches!(decision, PermissionDecision::Deny { .. }) {
                return decision;
            }
            if let Some(command) = command {
                for rule_decision in [
                    self.evaluate_bash_deny(command, names),
                    self.evaluate_bash_overlay_pattern(command, names),
                    self.evaluate_bash_pattern(command, names),
                    self.evaluate_command_class(command, names),
                ]
                .into_iter()
                .flatten()
                {
                    if matches!(rule_decision, PermissionDecision::Deny { .. }) {
                        return rule_decision;
                    }
                }
            }
            return decision;
        }
        if let Some((matched_name, mode)) = self.tool_name_mode(names) {
            // A rule the user wrote for this tool names an action, so it
            // outranks the command-class defaults below.
            return self.decision_for_mode(matched_name, mode);
        }
        if ToolTag::is_shell(tags)
            && let Some(command) = command
        {
            if let Some(decision) = self.evaluate_bash_deny(command, names) {
                return decision;
            }
            if let Some(decision) = self.evaluate_bash_overlay_pattern(command, names) {
                return decision;
            }
            if let Some(decision) = self.evaluate_bash_pattern(command, names) {
                return decision;
            }
            // The command classes are defaults: a command the user named under
            // `tools.rules` was answered above, and a command no rule names
            // lands on its class default.
            if let Some(decision) = self.evaluate_command_class(command, names) {
                return decision;
            }
        }
        // The read-only class default. Like the interaction default it is
        // ranked above `tools.default`, and below a rule naming the tool.
        if let Some(decision) = self.evaluate_read_only_class(names, tags) {
            return decision;
        }
        let name = names.first().copied().unwrap_or("tool");
        self.decision_for_mode(name, self.default_mode)
    }

    /// The configured mode for one of `names`, if the user named the tool or a
    /// default did. This is the map `tools.names` writes into.
    fn tool_name_mode<'a>(&self, names: &[&'a str]) -> Option<(&'a str, PermissionMode)> {
        names.iter().find_map(|name| {
            self.tool_modes
                .get(*name)
                .copied()
                .map(|mode| (*name, mode))
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
    /// `tools.rules.<shell tool>` entries like any other), so a command no
    /// pattern names falls to its class's rule — the built-in
    /// `allow` / `allow` / `deny` when the configuration carries none.
    /// A class left at `Auto` is not decided statically and reaches the
    /// approval model.
    fn evaluate_command_class(&self, command: &str, names: &[&str]) -> Option<PermissionDecision> {
        let normalized = command.trim();
        if normalized.is_empty() {
            return None;
        }
        let class = crate::command_class::command_classes(normalized)?;
        let configured = self
            .bash_overlay_rules
            .iter()
            .rev()
            .find(|rule| rule.applies_to(names) && matches!(rule.matcher, CommandPatternMatcher::Class(rule_class) if rule_class == class))
            .map(|rule| rule.mode);
        let mode = configured.unwrap_or_else(|| built_in_command_class_mode(class));
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

    fn evaluate_bash_pattern(&self, command: &str, names: &[&str]) -> Option<PermissionDecision> {
        let normalized = command.trim();
        if normalized.is_empty() {
            return None;
        }
        for rule in &self.bash_pattern_rules {
            if rule.applies_to(names) && rule.matches(normalized) {
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

    fn evaluate_bash_overlay_pattern(
        &self,
        command: &str,
        names: &[&str],
    ) -> Option<PermissionDecision> {
        let normalized = command.trim();
        if normalized.is_empty() {
            return None;
        }
        for rule in self.bash_overlay_rules.iter().rev() {
            if rule.applies_to(names) && rule.matches(normalized) {
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

    fn evaluate_bash_deny(&self, command: &str, names: &[&str]) -> Option<PermissionDecision> {
        let normalized = command.trim();
        if normalized.is_empty() {
            return None;
        }
        for rule in &self.bash_deny_rules {
            if rule.applies_to(names) && rule.matches(normalized) {
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
    #[error("unknown permission path marker `{alias}` in pattern `{pattern}`")]
    UnknownPathAlias { pattern: String, alias: String },
    #[error("permission path marker `{alias}` cannot be resolved for pattern `{pattern}`")]
    UnresolvedPathAlias { pattern: String, alias: String },
    #[error("invalid permission network rule `{pattern}`: {reason}")]
    InvalidNetworkRule { pattern: String, reason: String },
}

#[derive(Debug, Clone)]
/// Permission policy for network access.
pub struct NetworkPermissionPolicy {
    pub(crate) internet_default: PermissionMode,
    pub(crate) private_default: PermissionMode,
    pub(crate) loopback_default: PermissionMode,
    rules: Vec<NetworkPermissionRule>,
}

impl NetworkPermissionPolicy {
    pub fn new(default_mode: PermissionMode) -> Self {
        Self {
            internet_default: default_mode,
            private_default: default_mode,
            loopback_default: default_mode,
            rules: Vec::new(),
        }
    }

    pub fn allow_all() -> Self {
        Self::new(PermissionMode::Allow)
    }

    pub fn add_rule(
        &mut self,
        pattern: impl Into<String>,
        mode: PermissionMode,
    ) -> Result<(), PermissionConfigError> {
        self.rules.push(NetworkPermissionRule::new(pattern, mode)?);
        Ok(())
    }

    pub fn check_connect(&self, target: &NetworkTarget) -> PermissionDecision {
        for rule in self.rules.iter().rev() {
            if rule.matches(target) {
                return decide_from_mode(rule.mode, &rule.description);
            }
        }

        let (mode, summary) = match classify_network_target(target) {
            NetworkClass::Internet => (
                self.internet_default,
                "matched internet network default permission",
            ),
            NetworkClass::Private => (
                self.private_default,
                "matched private network default permission",
            ),
            NetworkClass::Loopback => (
                self.loopback_default,
                "matched loopback network default permission",
            ),
        };
        decide_from_mode(mode, summary)
    }
}

#[derive(Debug, Clone)]
struct NetworkPermissionRule {
    mode: PermissionMode,
    matcher: NetworkRuleMatcher,
    description: String,
}

impl NetworkPermissionRule {
    fn new(
        pattern: impl Into<String>,
        mode: PermissionMode,
    ) -> Result<Self, PermissionConfigError> {
        let pattern = pattern.into();
        let matcher = NetworkRuleMatcher::new(pattern.as_str())?;
        Ok(Self {
            mode,
            matcher,
            description: format!("matched network rule: {pattern}"),
        })
    }

    fn matches(&self, target: &NetworkTarget) -> bool {
        self.matcher.matches(target)
    }
}

#[derive(Debug, Clone)]
struct NetworkRuleMatcher {
    host: NetworkHostMatcher,
    port: NetworkPortMatcher,
}

impl NetworkRuleMatcher {
    fn new(pattern: &str) -> Result<Self, PermissionConfigError> {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            return Err(PermissionConfigError::InvalidNetworkRule {
                pattern: pattern.to_string(),
                reason: "empty pattern".to_string(),
            });
        }
        let (host, port) = split_network_rule_host_port(trimmed).map_err(|reason| {
            PermissionConfigError::InvalidNetworkRule {
                pattern: pattern.to_string(),
                reason,
            }
        })?;
        Ok(Self {
            host: NetworkHostMatcher::new(host).map_err(|reason| {
                PermissionConfigError::InvalidNetworkRule {
                    pattern: pattern.to_string(),
                    reason,
                }
            })?,
            port: NetworkPortMatcher::new(port).map_err(|reason| {
                PermissionConfigError::InvalidNetworkRule {
                    pattern: pattern.to_string(),
                    reason,
                }
            })?,
        })
    }

    fn matches(&self, target: &NetworkTarget) -> bool {
        self.host.matches(target.host()) && self.port.matches(target.port())
    }
}

#[derive(Debug, Clone)]
enum NetworkHostMatcher {
    Any,
    ExactIp(IpAddr),
    Cidr { base: IpAddr, prefix: u8 },
    Wildcard(WildcardPattern),
}

impl NetworkHostMatcher {
    fn new(host: &str) -> Result<Self, String> {
        let host = host.trim();
        if host.is_empty() || host == "*" {
            return Ok(Self::Any);
        }
        if let Some((addr, prefix)) = host.split_once('/') {
            let base = addr
                .parse::<IpAddr>()
                .map_err(|_| format!("invalid CIDR address `{addr}`"))?;
            let prefix = prefix
                .parse::<u8>()
                .map_err(|_| format!("invalid CIDR prefix `{prefix}`"))?;
            let max = match base {
                IpAddr::V4(_) => 32,
                IpAddr::V6(_) => 128,
            };
            if prefix > max {
                return Err(format!("CIDR prefix `{prefix}` exceeds {max}"));
            }
            return Ok(Self::Cidr { base, prefix });
        }
        if let Ok(addr) = host.parse::<IpAddr>() {
            return Ok(Self::ExactIp(addr));
        }
        Ok(Self::Wildcard(WildcardPattern::new(normalize_host(host))))
    }

    fn matches(&self, host: &str) -> bool {
        match self {
            Self::Any => true,
            Self::ExactIp(expected) => host
                .parse::<IpAddr>()
                .is_ok_and(|actual| &actual == expected),
            Self::Cidr { base, prefix } => host
                .parse::<IpAddr>()
                .is_ok_and(|actual| cidr_contains(*base, *prefix, actual)),
            Self::Wildcard(pattern) => pattern.matches(&normalize_host(host)),
        }
    }
}

#[derive(Debug, Clone)]
enum NetworkPortMatcher {
    Any,
    Exact(u16),
}

impl NetworkPortMatcher {
    fn new(port: Option<&str>) -> Result<Self, String> {
        let Some(port) = port.map(str::trim).filter(|port| !port.is_empty()) else {
            return Ok(Self::Any);
        };
        if port == "*" {
            return Ok(Self::Any);
        }
        port.parse::<u16>()
            .map(Self::Exact)
            .map_err(|_| format!("invalid port `{port}`"))
    }

    fn matches(&self, port: Option<u16>) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(expected) => port == Some(*expected),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkClass {
    Internet,
    Private,
    Loopback,
}

fn classify_network_target(target: &NetworkTarget) -> NetworkClass {
    let host = target.host();
    if host == "localhost" || host.ends_with(".localhost") {
        return NetworkClass::Loopback;
    }
    if let Ok(addr) = host.parse::<IpAddr>() {
        return classify_ip_addr(addr);
    }
    if !host.contains('.')
        || host.ends_with(".local")
        || host.ends_with(".lan")
        || host.ends_with(".internal")
        || host.ends_with(".corp")
        || host.ends_with(".home.arpa")
    {
        return NetworkClass::Private;
    }
    NetworkClass::Internet
}

fn classify_ip_addr(addr: IpAddr) -> NetworkClass {
    match addr {
        IpAddr::V4(addr) if addr.is_loopback() => NetworkClass::Loopback,
        IpAddr::V6(addr) if addr.is_loopback() => NetworkClass::Loopback,
        IpAddr::V4(addr)
            if addr.is_private()
                || addr.is_link_local()
                || addr.octets()[0] == 0
                || addr.octets()[0] >= 224 =>
        {
            NetworkClass::Private
        }
        IpAddr::V6(addr) if is_private_ipv6(addr) => NetworkClass::Private,
        _ => NetworkClass::Internet,
    }
}

fn is_private_ipv6(addr: Ipv6Addr) -> bool {
    addr.is_unique_local() || addr.is_unicast_link_local() || addr.is_unspecified()
}

fn split_network_rule_host_port(pattern: &str) -> Result<(&str, Option<&str>), String> {
    if let Some(rest) = pattern.strip_prefix('[')
        && let Some((host, tail)) = rest.split_once(']')
    {
        let port = tail.strip_prefix(':');
        return Ok((host, port));
    }

    if pattern.matches(':').count() == 1
        && let Some((host, port)) = pattern.rsplit_once(':')
    {
        return Ok((host, Some(port)));
    }

    Ok((pattern, None))
}

fn normalize_host(host: impl AsRef<str>) -> String {
    host.as_ref()
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn cidr_contains(base: IpAddr, prefix: u8, actual: IpAddr) -> bool {
    match (base, actual) {
        (IpAddr::V4(base), IpAddr::V4(actual)) => {
            cidr_contains_u32(ipv4_to_u32(base), prefix, ipv4_to_u32(actual))
        }
        (IpAddr::V6(base), IpAddr::V6(actual)) => {
            cidr_contains_u128(ipv6_to_u128(base), prefix, ipv6_to_u128(actual))
        }
        _ => false,
    }
}

fn cidr_contains_u32(base: u32, prefix: u8, actual: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    };
    (base & mask) == (actual & mask)
}

fn cidr_contains_u128(base: u128, prefix: u8, actual: u128) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    };
    (base & mask) == (actual & mask)
}

fn ipv4_to_u32(addr: Ipv4Addr) -> u32 {
    u32::from_be_bytes(addr.octets())
}

#[cfg(test)]
mod tests {
    use agena_domain::PermissionAction;

    use super::{PermissionMode, ToolPermissionPolicy, tool_action};
    use agena_domain::PermissionDecision;
    use agena_plugin_host::sdk::ToolTag;

    fn shell_tags() -> Vec<ToolTag> {
        vec![ToolTag::Shell]
    }

    #[test]
    fn shell_capability_applies_command_patterns_to_shell_runner() {
        let mut policy = ToolPermissionPolicy::new(PermissionMode::Ask);
        policy.add_bash_overlay_rule("git status", PermissionMode::Allow);
        policy.add_bash_overlay_rule("git push *", PermissionMode::Deny);

        assert!(matches!(
            policy.check_tool(
                "agena.shell.run",
                Some("git status"),
                shell_tags().as_slice()
            ),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            policy.check_tool(
                "agena.shell.run",
                Some("git push origin main"),
                shell_tags().as_slice(),
            ),
            PermissionDecision::Deny { .. }
        ));
        assert!(matches!(
            tool_action(
                "agena.shell.run",
                Some("git status"),
                shell_tags().as_slice(),
                Some(&policy),
            ),
            PermissionAction::Tool {
                qualifier: Some(_),
                ..
            }
        ));
    }
}

fn ipv6_to_u128(addr: Ipv6Addr) -> u128 {
    u128::from_be_bytes(addr.octets())
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
    Absolute(WildcardPattern),
}

impl PathPattern {
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
        if let Some(rest) = strip_path_alias(&normalized, "home") {
            return absolute_alias_pattern(&normalized, "home", home_dir(), rest);
        }
        if let Some(rest) = strip_path_alias(&normalized, "tmp") {
            return absolute_alias_pattern(&normalized, "tmp", Some(std::env::temp_dir()), rest);
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
                .is_some_and(|relative| pattern.matches(relative)),
            Self::Absolute(pattern) => pattern.matches(&ctx.absolute_norm),
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

fn absolute_alias_pattern(
    pattern: &str,
    alias: &str,
    root: Option<PathBuf>,
    rest: &str,
) -> Result<PathPattern, PermissionConfigError> {
    let Some(root) = root else {
        return Err(PermissionConfigError::UnresolvedPathAlias {
            pattern: pattern.to_string(),
            alias: alias.to_string(),
        });
    };
    let mut normalized = normalize_path_string(&root);
    let rest = rest.trim_start_matches('/');
    if !rest.is_empty() {
        normalized.push('/');
        normalized.push_str(rest);
    }
    Ok(PathPattern::Absolute(WildcardPattern::new(normalized)))
}

fn unknown_angle_alias(pattern: &str) -> Option<String> {
    let rest = pattern.strip_prefix('<')?;
    let (alias, _) = rest.split_once('>')?;
    Some(alias.to_string())
}

fn home_dir() -> Option<PathBuf> {
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
    in_workspace: bool,
}

impl MatchContext {
    fn new(workspace_root: &Path, target_path: &Path) -> Self {
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

        let in_workspace =
            absolute_norm == root_norm || absolute_norm.starts_with(&format!("{root_norm}/"));

        let workspace_relative_norm = if in_workspace {
            if absolute_norm == root_norm {
                Some(".".to_string())
            } else {
                Some(
                    absolute_norm
                        .trim_start_matches(&format!("{root_norm}/"))
                        .to_string(),
                )
            }
        } else {
            None
        };

        Self {
            absolute_norm,
            workspace_relative_norm,
            in_workspace,
        }
    }
}

fn normalize_path_string(path: &Path) -> String {
    let cleaned = path.clean();
    let mut out = cleaned.to_string_lossy().replace('\\', "/");
    while out.ends_with('/') && out.len() > 1 {
        out.pop();
    }
    if cfg!(windows) {
        out.make_ascii_lowercase();
    }
    out
}
