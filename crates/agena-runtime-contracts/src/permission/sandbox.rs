//! Serializable, backend-neutral resource policy for sandboxed subprocesses.
//!
//! A snapshot captures the complete principal, including all independent
//! ceilings, and resolves path aliases in the parent. It carries no credentials
//! and grants nothing on its own: launch paths still require normal approval.

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxPermissionSnapshot {
    pub blocked: bool,
    pub workspace_root: PathBuf,
    /// The base policy followed by every independent authority ceiling.
    pub path_layers: Vec<SandboxPathPolicy>,
    pub network_layers: Vec<SandboxNetworkPolicy>,
    /// Effective persisted decisions, ordered from global to session scope.
    #[serde(default)]
    pub persisted_rules: Vec<SandboxPersistedRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxPersistedRule {
    pub action: PermissionAction,
    pub mode: PermissionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxPathAccess {
    Read,
    Write,
    Any,
}

impl SandboxPathAccess {
    fn matches(self, access: AccessKind) -> bool {
        matches!(
            (self, access),
            (Self::Any, _) | (Self::Read, AccessKind::Read) | (Self::Write, AccessKind::Write)
        )
    }
}

impl From<AccessSelector> for SandboxPathAccess {
    fn from(value: AccessSelector) -> Self {
        match value {
            AccessSelector::Read => Self::Read,
            AccessSelector::Write => Self::Write,
            AccessSelector::Any => Self::Any,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxPathPolicy {
    pub workspace_read_default: PermissionMode,
    pub workspace_write_default: PermissionMode,
    pub external_read_default: PermissionMode,
    pub external_write_default: PermissionMode,
    /// Preserves the configuration's last-matching-rule-wins ordering.
    pub rules: Vec<SandboxPathRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxPathRule {
    pub access: SandboxPathAccess,
    pub mode: PermissionMode,
    pub patterns: Vec<SandboxPathPattern>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SandboxPathPattern {
    Absolute {
        pattern: String,
    },
    /// Relative matching preserves the special `.` spelling for the root.
    /// In particular `<workspace>/**` matches the same paths as the original
    /// permission policy rather than adopting a different glob dialect.
    Relative {
        root: PathBuf,
        pattern: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxNetworkPolicy {
    pub internet_default: PermissionMode,
    pub private_default: PermissionMode,
    pub loopback_default: PermissionMode,
    pub rules: Vec<SandboxNetworkRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxNetworkRule {
    pub pattern: String,
    pub mode: PermissionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxFilesystemRestrictions {
    /// Additional read-only boundaries inside an approved writable tree.
    pub readonly_paths: Vec<PathBuf>,
    /// Boundaries which must remain inaccessible even inside an allowed tree.
    pub denied_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SandboxPolicyError {
    #[error("sandbox execution principal is blocked")]
    Blocked,
    #[error("sandbox path must be absolute: {path:?}")]
    RelativePath { path: PathBuf },
    #[error("sandbox policy denies {access} access to {path:?}")]
    Denied { path: PathBuf, access: &'static str },
    #[error(
        "sandbox write access also permits reading {path:?}; approve read access before launching"
    )]
    ReadApprovalRequired { path: PathBuf },
    #[error(
        "sandbox cannot safely express path rule `{pattern}` within the requested paths: {reason}"
    )]
    Unsupported { pattern: String, reason: String },
}

impl SandboxPermissionSnapshot {
    pub(crate) fn from_policies<'a>(
        blocked: bool,
        workspace_root: &Path,
        paths: impl Iterator<Item = &'a PermissionPolicy>,
        network: impl Iterator<Item = &'a NetworkPermissionPolicy>,
    ) -> Self {
        let workspace_root = if workspace_root.as_os_str().is_empty() {
            PathBuf::new()
        } else {
            PathBuf::from(normalize_path_string(workspace_root))
        };
        let managed = if workspace_root.as_os_str().is_empty() {
            None
        } else {
            Some(agena_storage_layout::StorageLayout::current().workspace_dir(&workspace_root))
        };
        Self {
            blocked,
            path_layers: paths
                .map(|policy| SandboxPathPolicy {
                    workspace_read_default: policy.workspace_read_default,
                    workspace_write_default: policy.workspace_write_default,
                    external_read_default: policy.external_read_default,
                    external_write_default: policy.external_write_default,
                    rules: policy
                        .rules
                        .iter()
                        .map(|rule| {
                            let RuleMatcher::PathPattern(pattern) = &rule.matcher;
                            let relative = |root: Option<&Path>, pattern: &WildcardPattern| {
                                root.map(|root| SandboxPathPattern::Relative {
                                    root: PathBuf::from(normalize_path_string(root)),
                                    pattern: pattern.pattern.clone(),
                                })
                                .into_iter()
                                .collect()
                            };
                            let patterns = match pattern {
                                PathPattern::Workspace(pattern) => relative(
                                    (!workspace_root.as_os_str().is_empty())
                                        .then_some(workspace_root.as_path()),
                                    pattern,
                                ),
                                PathPattern::ManagedWorkspace(pattern) => {
                                    relative(managed.as_deref(), pattern)
                                }
                                PathPattern::Absolute(pattern) => {
                                    vec![SandboxPathPattern::Absolute {
                                        pattern: pattern.pattern.clone(),
                                    }]
                                }
                                PathPattern::AbsoluteAny(patterns) => patterns
                                    .iter()
                                    .map(|pattern| SandboxPathPattern::Absolute {
                                        pattern: pattern.pattern.clone(),
                                    })
                                    .collect(),
                            };
                            SandboxPathRule {
                                access: rule.selector.into(),
                                mode: rule.mode,
                                patterns,
                            }
                        })
                        .collect(),
                })
                .collect(),
            network_layers: network
                .map(|policy| SandboxNetworkPolicy {
                    internet_default: policy.internet_default,
                    private_default: policy.private_default,
                    loopback_default: policy.loopback_default,
                    rules: policy
                        .rules
                        .iter()
                        .map(|rule| SandboxNetworkRule {
                            pattern: rule.pattern.clone(),
                            mode: rule.mode,
                        })
                        .collect(),
                })
                .collect(),
            workspace_root,
            persisted_rules: Vec::new(),
        }
    }

    /// The host supplies already scope-ordered, non-revoked decisions. This
    /// never replaces the static policy or removes an authority ceiling.
    pub fn with_persisted_rules(mut self, rules: Vec<SandboxPersistedRule>) -> Self {
        self.persisted_rules = rules;
        self
    }

    pub fn authorize_path_access(&self, access: AccessKind, target: &Path) -> PermissionDecision {
        if self.blocked {
            return denied("execution principal is blocked");
        }
        if !target.is_absolute() {
            return denied("sandbox path access requires an absolute path");
        }
        let mut decision = PermissionDecision::Allow;
        for layer in &self.path_layers {
            decision = intersect(decision, layer.check(access, &self.workspace_root, target));
        }
        let target = normalize_path_string(target);
        self.apply_persisted(decision, |action| {
            matches!(action,
                PermissionAction::PathAccess { access_kind, workspace_root, target_path }
                    if access_kind == access_name(access)
                        && path_identity::equal(Path::new(workspace_root), &self.workspace_root)
                        && path_identity::equal(Path::new(target_path), Path::new(&target))
            )
        })
    }

    pub fn authorize_network_connect(&self, target: &NetworkTarget) -> PermissionDecision {
        if self.blocked {
            return denied("execution principal is blocked");
        }
        let mut decision = PermissionDecision::Allow;
        for layer in &self.network_layers {
            let mut policy = NetworkPermissionPolicy::new(layer.internet_default);
            policy.private_default = layer.private_default;
            policy.loopback_default = layer.loopback_default;
            for rule in &layer.rules {
                if let Err(error) = policy.add_rule(&rule.pattern, rule.mode) {
                    return denied(format!("invalid sandbox network policy: {error}"));
                }
            }
            decision = intersect(decision, policy.check_connect(target));
        }
        self.apply_persisted(decision, |action| matches!(action,
            PermissionAction::NetworkAccess { target: original, host, port }
                if original == target.original() && host == target.host() && *port == target.port()
        ))
    }

    /// Check a DNS result for an already-approved concrete hostname and port.
    /// Domain approval survives an internet default deny, while numeric IP
    /// fences and a transition into another network class retain authority.
    pub fn authorize_resolved_network_address(
        &self,
        original: &NetworkTarget,
        resolved: &NetworkTarget,
    ) -> PermissionDecision {
        if self.blocked {
            return denied("execution principal is blocked");
        }
        if original.port() != resolved.port() {
            return denied("resolved network address changed the approved port");
        }
        let Some(address) = numeric_host(resolved.host()) else {
            return denied("resolved network target must be a numeric IP address");
        };
        let resolved_text = match (address, resolved.port()) {
            (IpAddr::V4(address), Some(port)) => format!("{address}:{port}"),
            (IpAddr::V6(address), Some(port)) => format!("[{address}]:{port}"),
            (address, None) => address.to_string(),
        };
        let resolved: NetworkTarget = match resolved_text.parse() {
            Ok(target) => target,
            Err(error) => return denied(format!("invalid resolved network address: {error}")),
        };
        let original_decision = self.authorize_network_connect(original);
        if matches!(original_decision, PermissionDecision::Deny { .. }) {
            return original_decision;
        }
        let original_address = numeric_host(original.host());
        let original_class =
            original_address.map_or_else(|| classify_network_target(original), classify_ip_addr);
        let resolved_class = classify_ip_addr(address);
        let mut decision = if original_class == resolved_class {
            let mut decision = PermissionDecision::Allow;
            for layer in &self.network_layers {
                let mut address_decision = PermissionDecision::Allow;
                for rule in layer.rules.iter().rev() {
                    let matcher = match NetworkRuleMatcher::new(&rule.pattern) {
                        Ok(matcher) => matcher,
                        Err(error) => {
                            return denied(format!("invalid sandbox network policy: {error}"));
                        }
                    };
                    // Only concrete address rules add a DNS-result
                    // constraint. Hostname decisions already participated in
                    // approval of the original destination.
                    if matcher.matches(&resolved) {
                        address_decision =
                            decide_from_mode(rule.mode, "matched resolved IP address rule");
                        break;
                    }
                }
                decision = intersect(decision, address_decision);
            }
            if original_address == Some(address)
                && !matches!(decision, PermissionDecision::Deny { .. })
            {
                PermissionDecision::Allow
            } else {
                decision
            }
        } else {
            // DNS must not borrow hostname approval to cross into another
            // address class. Only an explicit resolved Allow is usable.
            self.authorize_network_connect(&resolved)
        };
        for rule in &self.persisted_rules {
            let PermissionAction::NetworkAccess { host, port, .. } = &rule.action else {
                continue;
            };
            if rule.mode != PermissionMode::Deny
                || *port != resolved.port()
                || numeric_host(host) != Some(address)
            {
                continue;
            }
            // Resolve scope overrides per stored action key; a separate URL
            // spelling cannot accidentally erase another key's active deny.
            let stored =
                self.apply_persisted(PermissionDecision::Allow, |action| action == &rule.action);
            if matches!(stored, PermissionDecision::Deny { .. }) {
                decision = intersect(decision, stored);
            }
        }
        decision
    }

    fn apply_persisted(
        &self,
        base: PermissionDecision,
        matches: impl Fn(&PermissionAction) -> bool,
    ) -> PermissionDecision {
        let static_allows = matches!(base, PermissionDecision::Allow);
        let static_denies = matches!(base, PermissionDecision::Deny { .. });
        let mut decision = base;
        for rule in &self.persisted_rules {
            if !matches(&rule.action)
                || (static_allows
                    && matches!(rule.mode, PermissionMode::Ask | PermissionMode::Auto))
                || (static_denies && rule.mode != PermissionMode::Deny)
            {
                continue;
            }
            decision = decide_from_mode(rule.mode, "matched persisted sandbox permission rule");
        }
        decision
    }

    /// Authorize concrete worker I/O against this invocation's approved roots.
    /// Approval at a root covers inherited Ask/Auto policy, while a narrower
    /// rule still creates its own denial/read-only boundary.
    pub fn check_granted_path(
        &self,
        access: AccessKind,
        target: &Path,
        reads: &[PathBuf],
        writes: &[PathBuf],
    ) -> PermissionDecision {
        let grants: Vec<&PathBuf> = match access {
            AccessKind::Read => reads.iter().chain(writes).collect(),
            AccessKind::Write => writes.iter().collect(),
        };
        if !target.is_absolute() || !grants.iter().any(|grant| within(target, grant)) {
            return denied("path is outside this invocation's approved sandbox roots");
        }
        let decision = self.authorize_path_access(access, target);
        if matches!(decision, PermissionDecision::Deny { .. }) {
            return decision;
        }
        match self.filesystem_restrictions(reads, writes) {
            Err(error) => denied(error.to_string()),
            Ok(restrictions) => {
                if restrictions
                    .denied_paths
                    .iter()
                    .any(|path| within(target, path))
                    || (access == AccessKind::Write
                        && restrictions
                            .readonly_paths
                            .iter()
                            .any(|path| within(target, path)))
                {
                    denied("a more specific sandbox path rule restricts this access")
                } else {
                    PermissionDecision::Allow
                }
            }
        }
    }

    /// Project finer restrictions inside launch grants. The arguments are
    /// canonical absolute paths whose access has already been approved by the
    /// normal tool pipeline. No default policy is converted into a new grant.
    ///
    /// Literal paths and complete subtrees can be represented on all MXC
    /// backends. Filename globs cannot: enumerating existing files would miss
    /// files created after launch, so such a rule returns `Unsupported`.
    pub fn filesystem_restrictions(
        &self,
        reads: &[PathBuf],
        writes: &[PathBuf],
    ) -> Result<SandboxFilesystemRestrictions, SandboxPolicyError> {
        if self.blocked {
            return Err(SandboxPolicyError::Blocked);
        }
        let reads = normalized_paths(reads)?;
        let writes = normalized_paths(writes)?;
        let all_reads: Vec<_> = reads.iter().chain(&writes).cloned().collect();
        for (paths, access) in [(&all_reads, AccessKind::Read), (&writes, AccessKind::Write)] {
            for path in paths {
                if matches!(
                    self.authorize_path_access(access, path),
                    PermissionDecision::Deny { .. }
                ) {
                    return Err(SandboxPolicyError::Denied {
                        path: path.clone(),
                        access: access_name(access),
                    });
                }
            }
        }
        for path in &writes {
            if !matches!(
                self.authorize_path_access(AccessKind::Read, path),
                PermissionDecision::Allow
            ) && !reads.iter().any(|read| within(path, read))
            {
                return Err(SandboxPolicyError::ReadApprovalRequired { path: path.clone() });
            }
        }
        let mut restrictions = SandboxFilesystemRestrictions::default();
        for layer in &self.path_layers {
            for (access, grants) in [(AccessKind::Read, &all_reads), (AccessKind::Write, &writes)] {
                for grant in grants {
                    // Existing regular-file grants have no descendants. A
                    // directory (or future directory) must not turn a literal
                    // allow at its root into permission for denied children.
                    if grant.is_file() {
                        continue;
                    }
                    let mode = layer.subtree_default(access, &self.workspace_root, grant);
                    if mode == PermissionMode::Deny
                        || (mode != PermissionMode::Allow
                            && matches!(
                                layer.check(access, &self.workspace_root, grant),
                                PermissionDecision::Allow
                            ))
                    {
                        return Err(SandboxPolicyError::Unsupported {
                            pattern: grant.to_string_lossy().into_owned(),
                            reason: "a literal directory grant does not approve its more restrictive descendant default; grant a subtree explicitly".into(),
                        });
                    }
                }
            }
            for (index, rule) in layer.rules.iter().enumerate() {
                if rule.mode == PermissionMode::Allow {
                    continue;
                }
                for pattern in &rule.patterns {
                    for access in [AccessKind::Read, AccessKind::Write] {
                        if !rule.access.matches(access) {
                            continue;
                        }
                        let grants = if access == AccessKind::Read {
                            &all_reads
                        } else {
                            &writes
                        };
                        let global_deny =
                            access == AccessKind::Read && rule.mode == PermissionMode::Deny;
                        if grants.is_empty() && !global_deny {
                            continue;
                        }
                        // Explicit read denials also cover platform runtime
                        // baselines and inherited ACL access outside grants.
                        let global_roots = [pattern.filesystem_root()];
                        let boundary =
                            pattern.boundary(if global_deny { &global_roots } else { grants })?;
                        let Some((path, recursive)) = boundary else {
                            continue;
                        };
                        let mode = if recursive {
                            layer.rules[index + 1..]
                                .iter()
                                .filter(|later| {
                                    later.access.matches(access)
                                        && later
                                            .patterns
                                            .iter()
                                            .any(|pattern| pattern.covers_subtree(&path))
                                })
                                .next_back()
                                .map_or(rule.mode, |later| later.mode)
                        } else {
                            rule.mode
                        };
                        if mode != PermissionMode::Allow {
                            self.add_boundary(
                                &mut restrictions,
                                access,
                                mode,
                                &path,
                                recursive,
                                grants,
                                global_deny,
                            )?;
                        }
                    }
                }
            }
            // A grant outside a workspace can contain that workspace, whose
            // more restrictive class defaults must still be enforced.
            if !self.workspace_root.as_os_str().is_empty() {
                for (access, mode, grants) in [
                    (AccessKind::Read, layer.workspace_read_default, &all_reads),
                    (AccessKind::Write, layer.workspace_write_default, &writes),
                ] {
                    if mode != PermissionMode::Allow
                        && grants.iter().any(|grant| {
                            self.workspace_root != *grant && within(&self.workspace_root, grant)
                        })
                    {
                        self.add_boundary(
                            &mut restrictions,
                            access,
                            mode,
                            &self.workspace_root,
                            true,
                            grants,
                            false,
                        )?;
                    }
                }
            }
        }
        for rule in &self.persisted_rules {
            let PermissionAction::PathAccess {
                access_kind,
                workspace_root,
                target_path,
            } = &rule.action
            else {
                continue;
            };
            if !path_identity::equal(Path::new(workspace_root), &self.workspace_root) {
                continue;
            }
            let access = match access_kind.as_str() {
                "read" => AccessKind::Read,
                "write" => AccessKind::Write,
                _ => continue,
            };
            let grants = if access == AccessKind::Read {
                &all_reads
            } else {
                &writes
            };
            let path = PathBuf::from(normalize_path_string(Path::new(target_path)));
            let global_deny = access == AccessKind::Read && rule.mode == PermissionMode::Deny;
            if rule.mode != PermissionMode::Allow
                && (global_deny || grants.iter().any(|grant| within(&path, grant)))
            {
                self.add_boundary(
                    &mut restrictions,
                    access,
                    rule.mode,
                    &path,
                    false,
                    grants,
                    global_deny,
                )?;
            }
        }
        restrictions.denied_paths.sort();
        restrictions.denied_paths.dedup();
        restrictions.readonly_paths.sort();
        restrictions.readonly_paths.dedup();
        restrictions.readonly_paths.retain(|path| {
            !restrictions
                .denied_paths
                .iter()
                .any(|denied| within(path, denied))
        });
        Ok(restrictions)
    }

    fn add_boundary(
        &self,
        result: &mut SandboxFilesystemRestrictions,
        access: AccessKind,
        rule_mode: PermissionMode,
        path: &Path,
        recursive: bool,
        grants: &[PathBuf],
        global_deny: bool,
    ) -> Result<(), SandboxPolicyError> {
        if !global_deny
            && !grants
                .iter()
                .any(|grant| within(path, grant) || within(grant, path))
        {
            return Ok(());
        }
        // A launch grant approves Ask/Auto at its own root and inherited
        // policy. A more specific descendant rule is a separate boundary.
        if rule_mode != PermissionMode::Deny && grants.iter().any(|grant| within(grant, path)) {
            return Ok(());
        }
        let decision = self.authorize_path_access(access, path);
        // A subtree cannot be proven safe by probing a synthetic filename:
        // another rule may allow that one name while denying the rest.
        let restricted = recursive || !matches!(decision, PermissionDecision::Allow);
        if !restricted {
            return Ok(());
        }
        // A denied parent with an explicit child grant has different overlap
        // semantics across OS backends. Refuse that ambiguous launch.
        if grants.iter().any(|grant| within(grant, path)) {
            return Err(SandboxPolicyError::Unsupported {
                pattern: path.to_string_lossy().into_owned(),
                reason: "an approved path overlaps a stricter ancestor or whole-tree restriction"
                    .into(),
            });
        }
        match access {
            AccessKind::Read => result.denied_paths.push(path.to_path_buf()),
            AccessKind::Write => result.readonly_paths.push(path.to_path_buf()),
        }
        Ok(())
    }
}

impl SandboxPathPolicy {
    fn subtree_default(
        &self,
        access: AccessKind,
        workspace_root: &Path,
        target: &Path,
    ) -> PermissionMode {
        if let Some(rule) = self.rules.iter().rev().find(|rule| {
            rule.access.matches(access)
                && rule
                    .patterns
                    .iter()
                    .any(|pattern| pattern.covers_subtree(target))
        }) {
            return rule.mode;
        }
        let in_workspace = !workspace_root.as_os_str().is_empty() && within(target, workspace_root);
        match (access, in_workspace) {
            (AccessKind::Read, true) => self.workspace_read_default,
            (AccessKind::Write, true) => self.workspace_write_default,
            (AccessKind::Read, false) => self.external_read_default,
            (AccessKind::Write, false) => self.external_write_default,
        }
    }

    fn check(
        &self,
        access: AccessKind,
        workspace_root: &Path,
        target: &Path,
    ) -> PermissionDecision {
        for rule in self.rules.iter().rev() {
            if rule.access.matches(access)
                && rule.patterns.iter().any(|pattern| pattern.matches(target))
            {
                return decide_from_mode(rule.mode, "matched sandbox path rule");
            }
        }
        let in_workspace = !workspace_root.as_os_str().is_empty() && within(target, workspace_root);
        let mode = match (access, in_workspace) {
            (AccessKind::Read, true) => self.workspace_read_default,
            (AccessKind::Write, true) => self.workspace_write_default,
            (AccessKind::Read, false) => self.external_read_default,
            (AccessKind::Write, false) => self.external_write_default,
        };
        decide_from_mode(mode, "matched sandbox path class default")
    }
}

impl SandboxPathPattern {
    fn filesystem_root(&self) -> PathBuf {
        let path = match self {
            Self::Absolute { pattern } => Path::new(pattern),
            Self::Relative { root, .. } => root.as_path(),
        };
        path.ancestors().last().unwrap_or(path).to_path_buf()
    }

    fn covers_subtree(&self, path: &Path) -> bool {
        let pattern = match self {
            Self::Absolute { pattern } | Self::Relative { pattern, .. } => pattern,
        };
        if !pattern.ends_with('*') {
            return false;
        }
        self.boundary(&[path.to_path_buf()])
            .ok()
            .flatten()
            .is_some_and(|(root, recursive)| recursive && within(path, &root))
    }

    fn matches(&self, target: &Path) -> bool {
        let absolute = normalize_path_string(target);
        match self {
            Self::Absolute { pattern } => WildcardPattern::new(pattern).matches_path(&absolute),
            Self::Relative { root, pattern } => {
                let root = normalize_path_string(root);
                relative_path_norm(Path::new(&absolute), Path::new(&root))
                    .is_some_and(|relative| WildcardPattern::new(pattern).matches_path(&relative))
            }
        }
    }

    fn boundary(&self, grants: &[PathBuf]) -> Result<Option<(PathBuf, bool)>, SandboxPolicyError> {
        let (root, pattern) = match self {
            Self::Absolute { pattern } => (None, pattern.as_str()),
            Self::Relative { root, pattern } => (Some(root.as_path()), pattern.as_str()),
        };
        let path_for = |pattern: &str| -> PathBuf {
            let path = match root {
                Some(root) if pattern.is_empty() || pattern == "." => root.to_path_buf(),
                Some(root) => root.join(pattern),
                None => PathBuf::from(pattern),
            };
            PathBuf::from(normalize_path_string(&path))
        };
        if !pattern.contains(['*', '?']) {
            let path = path_for(pattern);
            return Ok(grants
                .iter()
                .any(|grant| within(&path, grant) || within(grant, &path))
                .then_some((path, false)));
        }
        // `*` in Agena matches directory separators, so only a complete
        // trailing subtree has a portable filesystem-policy representation.
        let without_stars = pattern.trim_end_matches('*');
        if !without_stars.contains(['*', '?'])
            && (without_stars.ends_with('/') || (without_stars.is_empty() && root.is_some()))
        {
            let path = path_for(without_stars.trim_end_matches('/'));
            return Ok(grants
                .iter()
                .any(|grant| within(&path, grant) || within(grant, &path))
                .then_some((path, true)));
        }
        let prefix = pattern.split(['*', '?']).next().unwrap_or_default();
        let parent = prefix.rsplit_once('/').map_or("", |(parent, _)| parent);
        let affected = if root.is_none() && parent.is_empty() {
            PathBuf::from(std::path::MAIN_SEPARATOR_STR)
        } else {
            path_for(parent)
        };
        if grants
            .iter()
            .any(|grant| within(&affected, grant) || within(grant, &affected))
        {
            return Err(SandboxPolicyError::Unsupported {
                pattern: match root { Some(root) => root.join(pattern).to_string_lossy().into_owned(), None => pattern.into() },
                reason: "wildcard restrictions require enforcement for future files, not an enumeration of existing matches".into(),
            });
        }
        Ok(None)
    }
}

fn numeric_host(host: &str) -> Option<IpAddr> {
    let address = host.trim_matches(['[', ']']).parse::<IpAddr>().ok()?;
    Some(match address {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map_or(IpAddr::V6(address), IpAddr::V4),
        address => address,
    })
}

fn normalized_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, SandboxPolicyError> {
    paths
        .iter()
        .map(|path| {
            if !path.is_absolute() {
                return Err(SandboxPolicyError::RelativePath { path: path.clone() });
            }
            Ok(PathBuf::from(normalize_path_string(path)))
        })
        .collect()
}

fn within(path: &Path, root: &Path) -> bool {
    if root.as_os_str().is_empty() {
        return false;
    }
    let path = PathBuf::from(normalize_path_string(path));
    let root = PathBuf::from(normalize_path_string(root));
    path_identity::within(&path, &root)
}

fn access_name(access: AccessKind) -> &'static str {
    match access {
        AccessKind::Read => "read",
        AccessKind::Write => "write",
    }
}

fn denied(reason: impl Into<String>) -> PermissionDecision {
    PermissionDecision::Deny {
        reason: reason.into(),
    }
}

fn intersect(left: PermissionDecision, right: PermissionDecision) -> PermissionDecision {
    match (&left, &right) {
        (PermissionDecision::Deny { .. }, _) => left,
        (_, PermissionDecision::Deny { .. }) => right,
        (PermissionDecision::Ask { .. }, _) => left,
        (_, PermissionDecision::Ask { .. }) => right,
        (PermissionDecision::Auto { .. }, _) => left,
        (_, PermissionDecision::Auto { .. }) => right,
        _ => PermissionDecision::Allow,
    }
}

#[cfg(test)]
mod tests;
