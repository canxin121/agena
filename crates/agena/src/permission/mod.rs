mod request;
mod resolver;
mod store;

use globset::{Glob, GlobMatcher};
use path_clean::PathClean;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::plugin::sdk::{ToolTag, normalize_tool_tag_name};

pub use request::{
    DecisionTrace, DecisionTraceStep, PendingPermission, PermissionAction, PermissionReply,
    PermissionReplyKind, PermissionRequest, PermissionRiskLevel, PermissionScope, PolicySourceKind,
};
pub use resolver::{
    PermissionResolution, PermissionResolutionSource, resolve_permission_with_persisted_rule,
};
pub use store::{PersistedPermissionRule, decide_from_mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessSelector {
    Read,
    Write,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Clone)]
pub struct ToolPermissionPolicy {
    default_mode: PermissionMode,
    tag_modes: HashMap<String, PermissionMode>,
    tool_modes: HashMap<String, PermissionMode>,
    bash_pattern_rules: Vec<BashPatternRule>,
    bash_deny_rules: Vec<BashPatternRule>,
    bash_overlay_rules: Vec<BashPatternRule>,
}

#[derive(Debug, Clone)]
pub struct BashPatternRule {
    matcher: CommandPatternMatcher,
    pattern: String,
    mode: PermissionMode,
}

impl BashPatternRule {
    pub fn new(
        pattern: impl Into<String>,
        mode: PermissionMode,
    ) -> Result<Self, PermissionConfigError> {
        let pattern = pattern.into();
        let glob = Glob::new(&pattern).map_err(|source| PermissionConfigError::InvalidGlob {
            pattern: pattern.clone(),
            source,
        })?;
        Ok(Self {
            matcher: CommandPatternMatcher::Glob(glob.compile_matcher()),
            pattern,
            mode,
        })
    }

    pub fn new_wildcard(pattern: impl Into<String>, mode: PermissionMode) -> Self {
        let pattern = pattern.into();
        Self {
            matcher: CommandPatternMatcher::Wildcard(WildcardPattern::new(&pattern)),
            pattern,
            mode,
        }
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    fn matches(&self, input: &str) -> bool {
        self.matcher.matches(input)
    }
}

#[derive(Debug, Clone)]
enum CommandPatternMatcher {
    Glob(GlobMatcher),
    Wildcard(WildcardPattern),
}

impl CommandPatternMatcher {
    fn matches(&self, input: &str) -> bool {
        match self {
            Self::Glob(glob) => glob.is_match(input),
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
    policy: Option<&ToolPermissionPolicy>,
) -> PermissionAction {
    let qualifier = if tool_name == "bash" {
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

    pub fn with_default_mode(mut self, mode: PermissionMode) -> Self {
        self.default_mode = mode;
        self
    }

    pub fn with_tag_mode(mut self, tag: ToolTag, mode: PermissionMode) -> Self {
        self.tag_modes.insert(tag.as_str().to_string(), mode);
        self
    }

    pub fn with_tool_mode(mut self, tool_name: impl Into<String>, mode: PermissionMode) -> Self {
        self.tool_modes.insert(tool_name.into(), mode);
        self
    }

    /// Append a bash command pattern rule. Patterns use `globset` glob syntax
    /// against the literal command string (e.g. `git status`, `rm *`,
    /// `pnpm *`). Rules are evaluated in registration order; the first match
    /// wins. Bash-pattern rules apply *only* to `bash`
    /// override the per-tool default for that one invocation when matched.
    pub fn with_bash_pattern_rule(
        mut self,
        pattern: impl Into<String>,
        mode: PermissionMode,
    ) -> Result<Self, PermissionConfigError> {
        self.bash_pattern_rules
            .push(BashPatternRule::new(pattern, mode)?);
        Ok(self)
    }

    /// Append a bash command pattern that *unconditionally* denies execution
    /// — checked before everything else, including configured bash pattern rules and the
    /// per-tool override. Useful for a global blocklist (`rm -rf *`,
    /// `:(){:|:&};:`, etc.) that even an explicit `Ask` rule should not be
    /// able to whitelist.
    pub fn with_bash_deny_pattern(
        mut self,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        self.bash_deny_rules
            .push(BashPatternRule::new(pattern, PermissionMode::Deny)?);
        Ok(self)
    }

    /// Append an overlay bash command pattern rule using opencode-style
    /// wildcard semantics. These rules are evaluated after unconditional deny
    /// patterns but before the base bash pattern rules, and the last matching
    /// overlay rule wins.
    pub fn with_bash_overlay_rule(
        mut self,
        pattern: impl Into<String>,
        mode: PermissionMode,
    ) -> Self {
        self.bash_overlay_rules
            .push(BashPatternRule::new_wildcard(pattern, mode));
        self
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

    pub fn check_tool_name(&self, name: &str) -> PermissionDecision {
        self.check_tool(name, None, &[])
    }

    pub fn check_tool(
        &self,
        name: &str,
        command: Option<&str>,
        tags: &[ToolTag],
    ) -> PermissionDecision {
        if name == "bash"
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
        self.check_tool_mode_with_tags(name, tags)
    }

    fn check_tool_mode_with_tags(&self, name: &str, tags: &[ToolTag]) -> PermissionDecision {
        if let Some(mode) = self.tool_modes.get(name).copied() {
            return self.decision_for_mode(name, mode);
        }
        let matched = tags
            .iter()
            .filter_map(|tag| self.tag_modes.get(tag.as_str()).copied())
            .reduce(combine_permission_modes);
        let mode = matched.unwrap_or(self.default_mode);
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

pub fn combine_permission_modes(left: PermissionMode, right: PermissionMode) -> PermissionMode {
    match (left, right) {
        (PermissionMode::Deny, _) | (_, PermissionMode::Deny) => PermissionMode::Deny,
        (PermissionMode::Ask, _) | (_, PermissionMode::Ask) => PermissionMode::Ask,
        (PermissionMode::Allow, PermissionMode::Allow) => PermissionMode::Allow,
    }
}

pub fn normalize_tool_tag(tag: impl AsRef<str>) -> Option<String> {
    normalize_tool_tag_name(tag)
}

pub fn push_tool_tag(tags: &mut Vec<ToolTag>, tag: ToolTag) {
    if !tags.iter().any(|existing| existing == &tag) {
        tags.push(tag);
    }
}

#[derive(Debug, Error)]
pub enum PermissionConfigError {
    #[error("invalid permission glob pattern '{pattern}': {source}")]
    InvalidGlob {
        pattern: String,
        source: globset::Error,
    },
    #[error("unknown permission path marker `{alias}` in pattern `{pattern}`")]
    UnknownPathAlias { pattern: String, alias: String },
    #[error("permission path marker `{alias}` cannot be resolved for pattern `{pattern}`")]
    UnresolvedPathAlias { pattern: String, alias: String },
    #[error("invalid permission path access shorthand `{value}`")]
    InvalidPathAccessShorthand { value: String },
    #[error("invalid permission network rule `{pattern}`: {reason}")]
    InvalidNetworkRule { pattern: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkTarget {
    original: String,
    host: String,
    port: Option<u16>,
}

impl NetworkTarget {
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, NetworkTargetParseError> {
        let original = raw.as_ref().trim().to_string();
        if original.is_empty() {
            return Err(NetworkTargetParseError::Empty);
        }

        if let Ok(url) = url::Url::parse(original.as_str())
            && let Some(host) = url.host_str()
        {
            return Ok(Self {
                original,
                host: normalize_host(host),
                port: url.port_or_known_default(),
            });
        }

        let (host, port) = split_network_host_port(original.as_str())?;
        let host = normalize_host(host);
        if host.trim().is_empty() {
            return Err(NetworkTargetParseError::MissingHost(original));
        }
        Ok(Self {
            original,
            host,
            port,
        })
    }

    pub fn original(&self) -> &str {
        &self.original
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }

    pub fn display(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{port}", self.host),
            None => self.host.clone(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NetworkTargetParseError {
    #[error("network target must not be empty")]
    Empty,
    #[error("network target `{0}` is missing a host")]
    MissingHost(String),
    #[error("network target `{target}` has invalid port `{port}`")]
    InvalidPort { target: String, port: String },
}

#[derive(Debug, Clone)]
pub struct NetworkPermissionPolicy {
    internet_default: PermissionMode,
    private_default: PermissionMode,
    loopback_default: PermissionMode,
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

    pub fn with_internet_default(mut self, mode: PermissionMode) -> Self {
        self.internet_default = mode;
        self
    }

    pub fn with_private_default(mut self, mode: PermissionMode) -> Self {
        self.private_default = mode;
        self
    }

    pub fn with_loopback_default(mut self, mode: PermissionMode) -> Self {
        self.loopback_default = mode;
        self
    }

    pub fn with_rule(
        mut self,
        pattern: impl Into<String>,
        mode: PermissionMode,
    ) -> Result<Self, PermissionConfigError> {
        self.rules.push(NetworkPermissionRule::new(pattern, mode)?);
        Ok(self)
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

fn split_network_host_port(target: &str) -> Result<(&str, Option<u16>), NetworkTargetParseError> {
    if let Some(rest) = target.strip_prefix('[')
        && let Some((host, tail)) = rest.split_once(']')
    {
        let port = if let Some(port) = tail.strip_prefix(':') {
            parse_network_target_port(target, port)?
        } else {
            None
        };
        return Ok((host, port));
    }

    if target.matches(':').count() == 1
        && let Some((host, port)) = target.rsplit_once(':')
    {
        return Ok((host, parse_network_target_port(target, port)?));
    }

    Ok((target, None))
}

fn parse_network_target_port(
    target: &str,
    port: &str,
) -> Result<Option<u16>, NetworkTargetParseError> {
    let port = port.trim();
    if port.is_empty() || port == "*" {
        return Ok(None);
    }
    port.parse::<u16>()
        .map(Some)
        .map_err(|_| NetworkTargetParseError::InvalidPort {
            target: target.to_string(),
            port: port.to_string(),
        })
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

fn ipv6_to_u128(addr: Ipv6Addr) -> u128 {
    u128::from_be_bytes(addr.octets())
}

#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    workspace_read_default: PermissionMode,
    workspace_write_default: PermissionMode,
    external_read_default: PermissionMode,
    external_write_default: PermissionMode,
    rules: Vec<PermissionRule>,
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

    pub fn with_workspace_defaults(mut self, read: PermissionMode, write: PermissionMode) -> Self {
        self.workspace_read_default = read;
        self.workspace_write_default = write;
        self
    }

    pub fn with_workspace_read_default(mut self, mode: PermissionMode) -> Self {
        self.workspace_read_default = mode;
        self
    }

    pub fn with_workspace_write_default(mut self, mode: PermissionMode) -> Self {
        self.workspace_write_default = mode;
        self
    }

    pub fn with_external_defaults(mut self, read: PermissionMode, write: PermissionMode) -> Self {
        self.external_read_default = read;
        self.external_write_default = write;
        self
    }

    pub fn with_external_read_default(mut self, mode: PermissionMode) -> Self {
        self.external_read_default = mode;
        self
    }

    pub fn with_external_write_default(mut self, mode: PermissionMode) -> Self {
        self.external_write_default = mode;
        self
    }

    pub fn allow_all() -> Self {
        Self::new(PermissionMode::Allow, PermissionMode::Allow)
    }

    pub fn read_all_write_workspace_only() -> Self {
        Self {
            workspace_read_default: PermissionMode::Allow,
            workspace_write_default: PermissionMode::Deny,
            external_read_default: PermissionMode::Allow,
            external_write_default: PermissionMode::Deny,
            rules: vec![PermissionRule {
                selector: AccessSelector::Write,
                mode: PermissionMode::Allow,
                matcher: RuleMatcher::WorkspaceOnly,
                description: "allow write inside workspace".to_string(),
            }],
        }
    }

    pub fn with_absolute_glob_rule(
        mut self,
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        self.rules
            .push(PermissionRule::absolute_glob(selector, mode, pattern)?);
        Ok(self)
    }

    pub fn with_workspace_glob_rule(
        mut self,
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        self.rules
            .push(PermissionRule::workspace_glob(selector, mode, pattern)?);
        Ok(self)
    }

    pub fn with_rule(mut self, rule: PermissionRule) -> Self {
        self.rules.push(rule);
        self
    }

    pub fn with_path_pattern_rule(
        mut self,
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        self.rules
            .push(PermissionRule::path_pattern(selector, mode, pattern)?);
        Ok(self)
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
    pub fn workspace_only(selector: AccessSelector, mode: PermissionMode) -> Self {
        Self {
            selector,
            mode,
            matcher: RuleMatcher::WorkspaceOnly,
            description: "matched workspace-only rule".to_string(),
        }
    }

    pub fn absolute_glob(
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        let pattern = pattern.into();
        let matcher = compile_glob(&pattern)?;
        Ok(Self {
            selector,
            mode,
            matcher: RuleMatcher::AbsoluteGlob(matcher),
            description: format!("matched absolute path glob: {pattern}"),
        })
    }

    pub fn workspace_glob(
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        let pattern = pattern.into();
        let matcher = compile_glob(&pattern)?;
        Ok(Self {
            selector,
            mode,
            matcher: RuleMatcher::WorkspaceGlob(matcher),
            description: format!("matched workspace-relative glob: {pattern}"),
        })
    }

    pub fn path_wildcard(
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Self {
        let pattern = pattern.into();
        Self {
            selector,
            mode,
            matcher: RuleMatcher::PathWildcard(WildcardPattern::new(&pattern)),
            description: format!("matched path wildcard: {pattern}"),
        }
    }

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
    WorkspaceOnly,
    AbsoluteGlob(GlobMatcher),
    WorkspaceGlob(GlobMatcher),
    PathWildcard(WildcardPattern),
    PathPattern(PathPattern),
}

impl RuleMatcher {
    fn matches(&self, ctx: &MatchContext) -> bool {
        match self {
            Self::WorkspaceOnly => ctx.in_workspace,
            Self::AbsoluteGlob(glob) => glob.is_match(&ctx.absolute_norm),
            Self::WorkspaceGlob(glob) => ctx
                .workspace_relative_norm
                .as_ref()
                .is_some_and(|relative| glob.is_match(relative)),
            Self::PathWildcard(pattern) => pattern.matches(
                ctx.workspace_relative_norm
                    .as_deref()
                    .unwrap_or(ctx.absolute_norm.as_str()),
            ),
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

fn compile_glob(pattern: &str) -> Result<GlobMatcher, PermissionConfigError> {
    let compiled = Glob::new(pattern).map_err(|source| PermissionConfigError::InvalidGlob {
        pattern: pattern.to_string(),
        source,
    })?;
    Ok(compiled.compile_matcher())
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::message::{ApplyPatchToolInput, ReadToolInput, ToolPayloadInput};

    use super::{
        AccessKind, AccessSelector, NetworkPermissionPolicy, NetworkTarget, PermissionDecision,
        PermissionMode, PermissionPolicy, ToolPermissionPolicy, normalize_path_string,
    };

    trait ToolPayloadPolicyExt {
        fn check_tool_input(&self, input: &ToolPayloadInput) -> PermissionDecision;
    }

    impl ToolPayloadPolicyExt for ToolPermissionPolicy {
        fn check_tool_input(&self, input: &ToolPayloadInput) -> PermissionDecision {
            let command = match input {
                ToolPayloadInput::Bash(payload) => Some(payload.command.as_str()),
                _ => None,
            };
            let tags = match input {
                ToolPayloadInput::Read(_) => vec![
                    crate::plugin::sdk::ToolTag::ReadOnly,
                    crate::plugin::sdk::ToolTag::FilesystemRead,
                ],
                ToolPayloadInput::ApplyPatch(_) => vec![
                    crate::plugin::sdk::ToolTag::Mutating,
                    crate::plugin::sdk::ToolTag::FilesystemWrite,
                ],
                ToolPayloadInput::Bash(_) => vec![
                    crate::plugin::sdk::ToolTag::Mutating,
                    crate::plugin::sdk::ToolTag::Shell,
                ],
                _ => Vec::new(),
            };
            self.check_tool(input.tool_name(), command, &tags)
        }
    }

    #[test]
    fn workspace_paths_use_workspace_defaults() {
        let root = workspace_root();
        let target = root.join("src/main.rs");

        let policy = PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Allow)
            .with_external_defaults(PermissionMode::Deny, PermissionMode::Deny);

        assert_eq!(
            policy.check_access(AccessKind::Write, &root, &target),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn external_default_applies_to_non_workspace_paths() {
        let root = workspace_root();
        let target = external_file("denied/file.txt");

        let policy = PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Allow)
            .with_external_defaults(PermissionMode::Deny, PermissionMode::Deny);

        match policy.check_access(AccessKind::Read, &root, &target) {
            PermissionDecision::Deny { reason } => {
                assert!(reason.contains("external default read"));
            }
            other => panic!("expected deny decision, got {other:?}"),
        }
    }

    #[test]
    fn network_policy_applies_class_defaults() {
        let policy = NetworkPermissionPolicy::allow_all()
            .with_internet_default(PermissionMode::Ask)
            .with_private_default(PermissionMode::Deny)
            .with_loopback_default(PermissionMode::Deny);

        match policy.check_connect(&NetworkTarget::parse("https://example.com").unwrap()) {
            PermissionDecision::Ask { reason } => assert!(reason.contains("internet")),
            other => panic!("expected internet ask decision, got {other:?}"),
        }
        match policy.check_connect(&NetworkTarget::parse("10.0.0.2:8080").unwrap()) {
            PermissionDecision::Deny { reason } => assert!(reason.contains("private")),
            other => panic!("expected private deny decision, got {other:?}"),
        }
        match policy.check_connect(&NetworkTarget::parse("localhost:3000").unwrap()) {
            PermissionDecision::Deny { reason } => assert!(reason.contains("loopback")),
            other => panic!("expected loopback deny decision, got {other:?}"),
        }
    }

    #[test]
    fn network_rules_match_host_port_wildcard_and_cidr() {
        let policy = NetworkPermissionPolicy::allow_all()
            .with_internet_default(PermissionMode::Ask)
            .with_rule("github.com:443", PermissionMode::Allow)
            .expect("host rule compiles")
            .with_rule("*.corp.local:443", PermissionMode::Deny)
            .expect("wildcard rule compiles")
            .with_rule("10.0.0.0/8:*", PermissionMode::Deny)
            .expect("cidr rule compiles");

        assert_eq!(
            policy.check_connect(&NetworkTarget::parse("https://github.com").unwrap()),
            PermissionDecision::Allow
        );
        match policy.check_connect(&NetworkTarget::parse("api.corp.local:443").unwrap()) {
            PermissionDecision::Deny { reason } => assert!(reason.contains("*.corp.local:443")),
            other => panic!("expected wildcard deny decision, got {other:?}"),
        }
        match policy.check_connect(&NetworkTarget::parse("10.1.2.3:80").unwrap()) {
            PermissionDecision::Deny { reason } => assert!(reason.contains("10.0.0.0/8:*")),
            other => panic!("expected cidr deny decision, got {other:?}"),
        }
    }

    #[test]
    fn external_custom_glob_can_whitelist_specific_external_paths() {
        let root = workspace_root();
        let allowed_dir = external_dir("whitelist");
        let blocked_dir = external_dir("blocked");

        let policy = PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Allow)
            .with_external_defaults(PermissionMode::Deny, PermissionMode::Deny)
            .with_path_pattern_rule(
                AccessSelector::Write,
                PermissionMode::Allow,
                format!("{}/**", normalize_path_string(&allowed_dir)),
            )
            .expect("external glob should compile");

        assert_eq!(
            policy.check_access(AccessKind::Write, &root, &allowed_dir.join("ok.txt")),
            PermissionDecision::Allow
        );

        match policy.check_access(AccessKind::Write, &root, &blocked_dir.join("no.txt")) {
            PermissionDecision::Deny { .. } => {}
            other => panic!("expected deny decision, got {other:?}"),
        }
    }

    #[test]
    fn last_matching_rule_wins_for_external_path_overrides() {
        let root = workspace_root();
        let common = external_dir("policy");
        let allowed = common.join("allowed");

        let policy = PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Allow)
            .with_external_defaults(PermissionMode::Deny, PermissionMode::Deny)
            .with_path_pattern_rule(
                AccessSelector::Write,
                PermissionMode::Deny,
                format!("{}/**", normalize_path_string(&common)),
            )
            .expect("deny glob should compile")
            .with_path_pattern_rule(
                AccessSelector::Write,
                PermissionMode::Allow,
                format!("{}/**", normalize_path_string(&allowed)),
            )
            .expect("allow override glob should compile");

        assert_eq!(
            policy.check_access(AccessKind::Write, &root, &allowed.join("ok.txt")),
            PermissionDecision::Allow
        );

        match policy.check_access(AccessKind::Write, &root, &common.join("other/no.txt")) {
            PermissionDecision::Deny { .. } => {}
            other => panic!("expected deny decision, got {other:?}"),
        }
    }

    #[test]
    fn tool_permission_policy_uses_default_mode() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Ask);
        let input = ToolPayloadInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });

        match policy.check_tool_input(&input) {
            PermissionDecision::Ask { reason } => {
                assert!(reason.contains("read"));
            }
            other => panic!("expected ask decision, got {other:?}"),
        }
    }

    #[test]
    fn tool_permission_policy_supports_per_tool_overrides() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Deny)
            .with_tool_mode("read", PermissionMode::Allow)
            .with_tool_mode("apply_patch", PermissionMode::Ask);

        let read = ToolPayloadInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });
        let apply_patch = ToolPayloadInput::ApplyPatch(ApplyPatchToolInput {
            patch: "*** Begin Patch\n*** Add File: README.md\n+hello\n*** End Patch".to_string(),
        });

        assert_eq!(policy.check_tool_input(&read), PermissionDecision::Allow);

        match policy.check_tool_input(&apply_patch) {
            PermissionDecision::Ask { reason } => {
                assert!(reason.contains("apply_patch"));
            }
            other => panic!("expected ask decision, got {other:?}"),
        }
    }

    #[test]
    fn bash_pattern_rule_allows_matching_command_even_when_default_is_ask() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Allow)
            .with_tool_mode("bash", PermissionMode::Ask)
            .with_bash_pattern_rule("git *", PermissionMode::Allow)
            .expect("git glob compiles");

        let bash = ToolPayloadInput::Bash(crate::message::BashToolInput {
            command: "git status".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
            filesystem_effects: Vec::new(),
        });
        assert_eq!(policy.check_tool_input(&bash), PermissionDecision::Allow);

        let other = ToolPayloadInput::Bash(crate::message::BashToolInput {
            command: "make".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
            filesystem_effects: Vec::new(),
        });
        match policy.check_tool_input(&other) {
            PermissionDecision::Ask { reason } => assert!(reason.contains("bash")),
            other => panic!("expected ask decision, got {other:?}"),
        }
    }

    #[test]
    fn bash_pattern_rule_can_demand_confirmation_for_dangerous_command() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Allow)
            .with_bash_pattern_rule("rm *", PermissionMode::Ask)
            .expect("rm glob compiles");

        let bash = ToolPayloadInput::Bash(crate::message::BashToolInput {
            command: "rm -rf build".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
            filesystem_effects: Vec::new(),
        });
        match policy.check_tool_input(&bash) {
            PermissionDecision::Ask { reason } => assert!(reason.contains("`rm *`")),
            other => panic!("expected ask decision, got {other:?}"),
        }

        // Non-bash invocations are unaffected by bash pattern rules.
        let read = ToolPayloadInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });
        assert_eq!(policy.check_tool_input(&read), PermissionDecision::Allow);
    }

    #[test]
    fn bash_pattern_rule_first_match_wins() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Ask)
            .with_bash_pattern_rule("git push *", PermissionMode::Ask)
            .expect("first rule compiles")
            .with_bash_pattern_rule("git *", PermissionMode::Allow)
            .expect("second rule compiles");

        let push = ToolPayloadInput::Bash(crate::message::BashToolInput {
            command: "git push origin master".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
            filesystem_effects: Vec::new(),
        });
        match policy.check_tool_input(&push) {
            PermissionDecision::Ask { reason } => assert!(reason.contains("`git push *`")),
            other => panic!("expected ask decision, got {other:?}"),
        }

        let status = ToolPayloadInput::Bash(crate::message::BashToolInput {
            command: "git status".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
            filesystem_effects: Vec::new(),
        });
        assert_eq!(policy.check_tool_input(&status), PermissionDecision::Allow);
    }

    #[test]
    fn bash_without_matching_pattern_falls_through_to_tool_default() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Deny)
            .with_bash_pattern_rule("git *", PermissionMode::Allow)
            .expect("rule compiles");

        let bash = ToolPayloadInput::Bash(crate::message::BashToolInput {
            command: "make build".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
            filesystem_effects: Vec::new(),
        });
        match policy.check_tool_input(&bash) {
            PermissionDecision::Deny { reason } => assert!(reason.contains("bash")),
            other => panic!("expected deny decision, got {other:?}"),
        }
    }

    #[test]
    fn tag_defaults_combine_by_most_restrictive_mode() {
        let policy = ToolPermissionPolicy::allow_all()
            .with_tag_mode(crate::plugin::sdk::ToolTag::ReadOnly, PermissionMode::Allow)
            .with_tag_mode(crate::plugin::sdk::ToolTag::Network, PermissionMode::Ask)
            .with_tag_mode(
                crate::plugin::sdk::ToolTag::PrivateNetwork,
                PermissionMode::Deny,
            );

        assert_eq!(
            policy.check_tool(
                "plugin_paths",
                None,
                &[
                    crate::plugin::sdk::ToolTag::ReadOnly,
                    crate::plugin::sdk::ToolTag::Network,
                ]
            ),
            PermissionDecision::Ask {
                reason: "tool 'plugin_paths' requires confirmation by policy".to_string()
            }
        );
        match policy.check_tool(
            "plugin_paths",
            None,
            &[
                crate::plugin::sdk::ToolTag::Network,
                crate::plugin::sdk::ToolTag::PrivateNetwork,
            ],
        ) {
            PermissionDecision::Deny { reason } => assert!(reason.contains("plugin_paths")),
            other => panic!("expected deny decision, got {other:?}"),
        }
    }

    #[test]
    fn bash_deny_pattern_overrides_allow_rule() {
        let policy = ToolPermissionPolicy::allow_all()
            .with_bash_pattern_rule("rm *", PermissionMode::Allow)
            .expect("rm allow rule compiles")
            .with_bash_deny_pattern("rm -rf /*")
            .expect("deny pattern compiles");

        let dangerous = ToolPayloadInput::Bash(crate::message::BashToolInput {
            command: "rm -rf /tmp/oops".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
            filesystem_effects: Vec::new(),
        });
        match policy.check_tool_input(&dangerous) {
            PermissionDecision::Deny { reason } => {
                assert!(reason.contains("deny pattern"));
            }
            other => panic!("expected unconditional Deny, got {other:?}"),
        }

        // A non-matching command still flows through the normal pipeline.
        let safe = ToolPayloadInput::Bash(crate::message::BashToolInput {
            command: "rm tmpfile".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
            filesystem_effects: Vec::new(),
        });
        assert_eq!(policy.check_tool_input(&safe), PermissionDecision::Allow);
    }

    fn workspace_root() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\workspace\repo")
        } else {
            PathBuf::from("/workspace/repo")
        }
    }

    fn external_dir(suffix: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"D:\external").join(suffix)
        } else {
            PathBuf::from("/external").join(suffix)
        }
    }

    fn external_file(suffix: &str) -> PathBuf {
        external_dir("").join(Path::new(suffix))
    }
}
