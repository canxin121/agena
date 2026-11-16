//! Most-specific write boundaries inside already approved worker resources.

use super::*;

impl SandboxPermissionSnapshot {
    pub(super) fn scoped_write_boundaries(
        &self,
        reads: &[PathBuf],
        writes: &[PathBuf],
        denied_paths: &[PathBuf],
    ) -> Result<(Vec<PathBuf>, Vec<PathBuf>), SandboxPolicyError> {
        if writes.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut candidates = writes.to_vec();
        if !self.workspace_root.as_os_str().is_empty() {
            candidates.push(self.workspace_root.clone());
        }
        for layer in &self.path_layers {
            for rule in &layer.rules {
                if !rule.access.matches(AccessKind::Write) {
                    continue;
                }
                for pattern in &rule.patterns {
                    if let Some((path, _)) = pattern.boundary(writes)? {
                        candidates.push(path);
                    }
                }
            }
        }
        let mut persisted = Vec::new();
        for rule in &self.persisted_rules {
            let PermissionAction::PathAccess {
                access_kind,
                workspace_root,
                target_path,
            } = &rule.action
            else {
                continue;
            };
            if access_kind != "write"
                || !path_identity::equal(Path::new(workspace_root), &self.workspace_root)
            {
                continue;
            }
            let path = PathBuf::from(normalize_path_string(Path::new(target_path)));
            candidates.push(path.clone());
            let current = self.authorize_path_access(AccessKind::Write, &path);
            let restricts = match rule.mode {
                PermissionMode::Deny => matches!(current, PermissionDecision::Deny { .. }),
                PermissionMode::Ask | PermissionMode::Auto => matches!(
                    current,
                    PermissionDecision::Ask { .. } | PermissionDecision::Auto { .. }
                ),
                PermissionMode::Allow => false,
            };
            if restricts {
                persisted.push((path, rule.mode));
            }
        }
        let mut candidates = normalized_paths(&candidates)?;
        candidates.retain(|path| writes.iter().any(|grant| within(path, grant)));
        candidates.sort_by(|left, right| {
            left.components()
                .count()
                .cmp(&right.components().count())
                .then(left.cmp(right))
        });
        candidates.dedup_by(|left, right| path_identity::equal(left, right));

        let mut readonly = Vec::new();
        let mut readwrite = Vec::new();
        for path in candidates {
            if denied_paths.iter().any(|denied| within(&path, denied)) {
                continue;
            }
            let allowed = self.scoped_access_allowed(AccessKind::Read, &path, reads)?
                && self.scoped_access_allowed(AccessKind::Write, &path, writes)?
                && persisted.iter().all(|(boundary, mode)| {
                    !within(&path, boundary)
                        || (*mode != PermissionMode::Deny
                            && writes
                                .iter()
                                .any(|grant| within(&path, grant) && within(grant, boundary)))
                });
            if !allowed {
                if writes
                    .iter()
                    .any(|grant| path_identity::equal(grant, &path))
                {
                    return Err(SandboxPolicyError::Denied {
                        path,
                        access: "write",
                    });
                }
                readonly.push(path);
            } else if readonly.iter().any(|boundary| within(&path, boundary)) {
                // Restoring a configured descendant only refines a root that
                // the host already approved; it never expands the invocation.
                readwrite.push(path);
            }
        }
        Ok((readonly, readwrite))
    }

    fn scoped_access_allowed(
        &self,
        access: AccessKind,
        path: &Path,
        grants: &[PathBuf],
    ) -> Result<bool, SandboxPolicyError> {
        if matches!(
            self.authorize_path_access(access, path),
            PermissionDecision::Deny { .. }
        ) {
            return Ok(false);
        }
        for layer in &self.path_layers {
            if !layer.permits_approved_subtree(access, &self.workspace_root, path, grants)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl SandboxPathPolicy {
    fn permits_approved_subtree(
        &self,
        access: AccessKind,
        workspace: &Path,
        path: &Path,
        grants: &[PathBuf],
    ) -> Result<bool, SandboxPolicyError> {
        for descendants in [false, true] {
            if descendants && path.is_file() {
                continue;
            }
            let covers = |pattern: &SandboxPathPattern| {
                if descendants {
                    pattern.covers_subtree(path)
                } else {
                    pattern.matches(path)
                }
            };
            let rule = self
                .rules
                .iter()
                .rev()
                .find(|rule| rule.access.matches(access) && rule.patterns.iter().any(covers));
            let in_workspace = !workspace.as_os_str().is_empty() && within(path, workspace);
            let mode = rule.map_or_else(
                || match (access, in_workspace) {
                    (AccessKind::Read, true) => self.workspace_read_default,
                    (AccessKind::Write, true) => self.workspace_write_default,
                    (AccessKind::Read, false) => self.external_read_default,
                    (AccessKind::Write, false) => self.external_write_default,
                },
                |rule| rule.mode,
            );
            match mode {
                PermissionMode::Allow => continue,
                PermissionMode::Deny => return Ok(false),
                PermissionMode::Ask | PermissionMode::Auto => {}
            }
            let approved = if let Some(rule) = rule {
                let mut approved = false;
                for pattern in rule.patterns.iter().filter(|pattern| covers(pattern)) {
                    if let Some((boundary, _)) = pattern.boundary(&[path.to_path_buf()])? {
                        approved |= grants
                            .iter()
                            .any(|grant| within(path, grant) && within(grant, &boundary));
                    }
                }
                approved
            } else {
                grants
                    .iter()
                    .any(|grant| within(path, grant) && (!in_workspace || within(grant, workspace)))
            };
            if !approved {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
