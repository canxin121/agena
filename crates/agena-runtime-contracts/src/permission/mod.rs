mod resolver;

use agena_domain::{
    AccessKind, AccessSelector, NetworkTarget, PermissionAction, PermissionDecision,
    PermissionMode, decide_from_mode,
};
use path_clean::PathClean;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use thiserror::Error;

use agena_plugin_host::sdk::ToolTag;

pub use resolver::resolve_permission_with_persisted_rules;

#[derive(Debug, Clone)]
pub struct ToolPermissionPolicy {
    pub(crate) default_mode: PermissionMode,
    pub(crate) tag_modes: HashMap<String, PermissionMode>,
    pub(crate) tool_modes: HashMap<String, PermissionMode>,
    pub(crate) bash_pattern_rules: Vec<BashPatternRule>,
    pub(crate) bash_deny_rules: Vec<BashPatternRule>,
    pub(crate) bash_overlay_rules: Vec<BashPatternRule>,
}

#[derive(Debug, Clone)]
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

    fn matches(&self, input: &str) -> bool {
        self.matcher.matches(input)
    }
}

#[derive(Debug, Clone)]
enum CommandPatternMatcher {
    Wildcard(WildcardPattern),
}

impl CommandPatternMatcher {
    fn matches(&self, input: &str) -> bool {
        match self {
            Self::Wildcard(pattern) => pattern.matches(input),
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

pub fn tool_action(
    tool_name: &str,
    command: Option<&str>,
    tags: &[ToolTag],
    policy: Option<&ToolPermissionPolicy>,
) -> PermissionAction {
    let qualifier = if is_shell_tool(&[tool_name], tags) {
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
            default_mode,
            tag_modes: HashMap::new(),
            tool_modes: HashMap::new(),
            bash_pattern_rules: Vec::new(),
            bash_deny_rules: Vec::new(),
            bash_overlay_rules: Vec::new(),
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
        if is_shell_tool(names, tags)
            && let Some(command) = command
        {
            if let Some(decision) = self.evaluate_bash_deny(command) {
                return decision;
            }
            if let Some(decision) = self.evaluate_bash_overlay_pattern(command) {
                return decision;
            }
            if let Some(decision) = self.evaluate_bash_pattern(command) {
                return decision;
            }
        }
        self.check_tool_mode_with_names(names, tags)
    }

    pub fn check_tool(
        &self,
        name: &str,
        command: Option<&str>,
        tags: &[ToolTag],
    ) -> PermissionDecision {
        self.check_tool_with_names(&[name], command, tags)
    }

    fn check_tool_mode_with_names(&self, names: &[&str], tags: &[ToolTag]) -> PermissionDecision {
        if let Some((matched_name, mode)) = names.iter().find_map(|name| {
            self.tool_modes
                .get(*name)
                .copied()
                .map(|mode| (*name, mode))
        }) {
            return self.decision_for_mode(matched_name, mode);
        }
        let matched = tags
            .iter()
            .filter_map(|tag| self.tag_modes.get(tag.as_ref()).copied())
            .reduce(combine_permission_modes);
        let mode = matched.unwrap_or(self.default_mode);
        let name = names.first().copied().unwrap_or("tool");
        self.decision_for_mode(name, mode)
    }

    fn decision_for_mode(&self, name: &str, mode: PermissionMode) -> PermissionDecision {
        match mode {
            PermissionMode::Allow => PermissionDecision::Allow,
            PermissionMode::Ask => PermissionDecision::Ask {
                reason: format!("tool '{name}' requires confirmation by policy"),
            },
            PermissionMode::Deny => PermissionDecision::Deny {
                reason: format!("tool '{name}' denied by policy"),
            },
        }
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
            if rule.matches(normalized) {
                let decision = match rule.mode {
                    PermissionMode::Allow => PermissionDecision::Allow,
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

fn is_shell_tool(names: &[&str], tags: &[ToolTag]) -> bool {
    names.contains(&"bash") || tags.iter().any(|tag| matches!(tag, ToolTag::Shell))
}

pub fn combine_permission_modes(left: PermissionMode, right: PermissionMode) -> PermissionMode {
    match (left, right) {
        (PermissionMode::Deny, _) | (_, PermissionMode::Deny) => PermissionMode::Deny,
        (PermissionMode::Ask, _) | (_, PermissionMode::Ask) => PermissionMode::Ask,
        (PermissionMode::Allow, PermissionMode::Allow) => PermissionMode::Allow,
    }
}

#[derive(Debug, Error)]
pub enum PermissionConfigError {
    #[error("unknown permission path marker `{alias}` in pattern `{pattern}`")]
    UnknownPathAlias { pattern: String, alias: String },
    #[error("permission path marker `{alias}` cannot be resolved for pattern `{pattern}`")]
    UnresolvedPathAlias { pattern: String, alias: String },
    #[error("invalid permission path access shorthand `{value}`")]
    InvalidPathAccessShorthand { value: String },
    #[error("invalid permission network rule `{pattern}`: {reason}")]
    InvalidNetworkRule { pattern: String, reason: String },
}

#[derive(Debug, Clone)]
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

    use super::{PermissionMode, ToolPermissionPolicy, ToolTag, tool_action};
    use agena_domain::PermissionDecision;

    #[test]
    fn shell_tag_applies_command_patterns_to_shell_runner() {
        let mut policy = ToolPermissionPolicy::new(PermissionMode::Ask);
        policy.add_bash_overlay_rule("git status", PermissionMode::Allow);
        policy.add_bash_overlay_rule("git push *", PermissionMode::Deny);

        assert!(matches!(
            policy.check_tool("agena.shell.run", Some("git status"), &[ToolTag::Shell],),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            policy.check_tool(
                "agena.shell.run",
                Some("git push origin main"),
                &[ToolTag::Shell],
            ),
            PermissionDecision::Deny { .. }
        ));
        assert!(matches!(
            tool_action(
                "agena.shell.run",
                Some("git status"),
                &[ToolTag::Shell],
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
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(workspace_root)
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
