//! Declarative concrete path permissions.

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};

use crate::PathAccessModes;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Defaults authorize requested resources; a rule names a concrete file or
/// directory tree. Explicit denied rules cannot be reopened by descendants.
pub struct PathPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathAccessModes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<PathAccessModes>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub rules: IndexMap<String, PathAccessModes>,
}

impl<'de> Deserialize<'de> for PathPermissionConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize, Default)]
        #[serde(default, deny_unknown_fields)]
        struct Input {
            workspace: Option<PathAccessModes>,
            external: Option<PathAccessModes>,
            rules: IndexMap<String, serde_json::Value>,
        }

        let input = Input::deserialize(deserializer)?;
        let mut parsed = Vec::new();
        for (path, raw) in input.rules {
            let legacy = raw.is_object();
            let modes: PathAccessModes = serde_json::from_value(raw).map_err(|error| {
                serde::de::Error::custom(format!(
                    "invalid permission.path.rules[{path:?}]: {error}"
                ))
            })?;
            let canonical = canonical_permission_path(&path).map_err(|reason| {
                serde::de::Error::custom(format!(
                    "invalid permission.path.rules[{path:?}]: {reason}"
                ))
            })?;
            let subtree = path.replace('\\', "/").ends_with("/**");
            parsed.push((path, canonical, modes, legacy, subtree));
        }
        for (path, canonical, modes, legacy, subtree) in &parsed {
            if !legacy || modes.sandbox_mode() == Ok(crate::PathPermissionMode::Deny) {
                continue;
            }
            let paired = parsed
                .iter()
                .any(|(_, other, other_modes, _, other_subtree)| {
                    canonical == other && subtree != other_subtree && modes == other_modes
                });
            if !paired {
                return Err(serde::de::Error::custom(format!(
                    "legacy permission.path.rules[{path:?}] cannot be migrated without changing its scope; a directory rule now includes descendants. Explicitly replace it with readonly or readwrite after reviewing the path, or provide matching directory and directory/** legacy rules"
                )));
            }
        }
        let mut config = Self {
            workspace: input.workspace,
            external: input.external,
            rules: parsed
                .into_iter()
                .map(|(path, _, modes, _, _)| (path, modes))
                .collect(),
        };
        config.normalize_rules().map_err(serde::de::Error::custom)?;
        Ok(config)
    }
}

impl PathPermissionConfig {
    pub fn is_empty(&self) -> bool {
        self.workspace.is_none() && self.external.is_none() && self.rules.is_empty()
    }

    /// Validate programmatically constructed settings as well as deserialized
    /// settings. Storage callers can enforce this without a runtime dependency.
    pub fn validate(&self) -> Result<(), String> {
        let mut config = self.clone();
        config.normalize_rules()
    }

    /// Collapse the legacy directory/root + /** spelling only when both
    /// entries agree. A conflicting duplicate is an error, never last-wins.
    pub fn normalize_rules(&mut self) -> Result<(), String> {
        for (name, modes) in [("workspace", &self.workspace), ("external", &self.external)] {
            if let Some(modes) = modes {
                modes
                    .sandbox_mode()
                    .map_err(|reason| format!("invalid permission.path.{name}: {reason}"))?;
            }
        }
        let mut normalized = IndexMap::<String, PathAccessModes>::new();
        for (path, modes) in &self.rules {
            let canonical = canonical_permission_path(path)
                .map_err(|reason| format!("invalid permission.path.rules[{path:?}]: {reason}"))?;
            modes
                .sandbox_mode()
                .map_err(|reason| format!("invalid permission.path.rules[{path:?}]: {reason}"))?;
            if let Some(existing) = normalized.get(&canonical) {
                if existing != modes {
                    return Err(format!(
                        "conflicting permission.path.rules entries resolve to {canonical:?}; use one concrete path with one mode"
                    ));
                }
            } else {
                normalized.insert(canonical, modes.clone());
            }
        }
        let mut ordered = normalized.iter().collect::<Vec<_>>();
        ordered.sort_by(|(left, _), (right, _)| left.len().cmp(&right.len()).then(left.cmp(right)));
        for (index, (parent, parent_modes)) in ordered.iter().enumerate() {
            let parent_mode = parent_modes.sandbox_mode().expect("validated mode");
            for (child, child_modes) in ordered.iter().skip(index + 1) {
                let child_mode = child_modes.sandbox_mode().expect("validated mode");
                if !child.starts_with(&format!("{}/", parent.trim_end_matches('/')))
                    || parent_mode == child_mode
                {
                    continue;
                }
                if parent_mode == crate::PathPermissionMode::Deny {
                    return Err(format!(
                        "invalid permission.path.rules[{child:?}]: an explicitly denied directory cannot contain a granted child; conflicts with {parent:?}"
                    ));
                }
            }
        }
        self.rules = normalized;
        Ok(())
    }

    pub fn merge_from(&mut self, overlay: Self) {
        if let Some(workspace) = overlay.workspace {
            self.workspace = Some(workspace);
        }
        if let Some(external) = overlay.external {
            self.external = Some(external);
        }
        // Concrete modes are atomic. An override replaces the whole value,
        // rather than inheriting one half of a legacy read/write pair.
        self.rules.extend(overlay.rules);
    }
}

/// Validate and normalize one resource path without accessing the filesystem.
/// Runtime identity checks additionally resolve aliases and symlinks before
/// launch. No file-name glob is representable by this public rule language.
pub fn canonical_permission_path(pattern: &str) -> Result<String, String> {
    if pattern.is_empty() || pattern.trim() != pattern {
        return Err("a concrete path is required, without surrounding whitespace".into());
    }
    if pattern.chars().any(char::is_control) {
        return Err("control characters are not allowed in a permission path".into());
    }
    let normalized = pattern.replace('\\', "/");
    let normalized = normalized
        .strip_suffix("**")
        .filter(|path| path.ends_with('/'))
        .unwrap_or(&normalized);
    if normalized.contains(['*', '?']) {
        return Err(
            "file-name wildcards are unsupported by MXC path rules; name a concrete file or directory (directories include descendants)"
                .into(),
        );
    }
    if normalized.starts_with('~') {
        return Err("use <home> instead of ~ in a permission path".into());
    }
    let (prefix, suffix) = if normalized.starts_with('<') {
        let (alias, suffix) = normalized.split_once('>').ok_or_else(|| {
            "unterminated path alias; use <workspace>, <cwd>, <home>, <tmp>, or <state>".to_string()
        })?;
        let prefix = match alias {
            "<workspace" | "<cwd" => "<workspace>",
            "<home" => "<home>",
            "<tmp" => "<tmp>",
            "<state" => "<state>",
            _ => return Err(format!("unknown path alias {alias}>")),
        };
        if !suffix.is_empty() && !suffix.starts_with('/') {
            return Err("a path alias must be followed by / or the end of the path".into());
        }
        (prefix.to_owned(), suffix)
    } else if normalized.starts_with("//") {
        return Err("network-share paths are not supported; use a local concrete path".into());
    } else if normalized.starts_with('/') {
        ("/".to_string(), normalized)
    } else if normalized.as_bytes().get(1) == Some(&b':') {
        if !normalized.as_bytes()[0].is_ascii_alphabetic()
            || normalized.as_bytes().get(2) != Some(&b'/')
        {
            return Err("Windows paths must be absolute, for example C:/work/output".into());
        }
        (
            format!(
                "{}:/",
                normalized.as_bytes()[0].to_ascii_uppercase() as char
            ),
            &normalized[3..],
        )
    } else {
        ("<workspace>".to_string(), normalized)
    };
    let mut parts = Vec::new();
    for part in suffix.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(
                "parent traversal (..) is not allowed; name the intended absolute path or alias"
                    .into(),
            ),
            part if part.contains(['<', '>', ':']) => {
                return Err("path aliases are only allowed at the beginning, and alternate data streams are unsupported".into());
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Ok(prefix);
    }
    if prefix.ends_with('/') {
        Ok(format!("{prefix}{}", parts.join("/")))
    } else {
        Ok(format!("{prefix}/{}", parts.join("/")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_directories_and_safe_legacy_suffixes_share_one_key() {
        for (input, expected) in [
            ("input", "<workspace>/input"),
            ("./input/", "<workspace>/input"),
            ("<cwd>/input", "<workspace>/input"),
            ("<workspace>/input/**", "<workspace>/input"),
            ("<workspace>", "<workspace>"),
            ("<home>/.ssh", "<home>/.ssh"),
            ("/srv/project/output", "/srv/project/output"),
            ("c:\\work\\output", "C:/work/output"),
            ("/", "/"),
            ("/**", "/"),
            ("c:\\", "C:/"),
            ("C:/**", "C:/"),
        ] {
            assert_eq!(canonical_permission_path(input).unwrap(), expected);
        }
        let value: PathPermissionConfig = serde_json::from_value(serde_json::json!({
            "rules": {
                "<workspace>/input": "readonly",
                "<workspace>/input/**": {"read":"allow","write":"deny"}
            }
        }))
        .unwrap();
        assert_eq!(value.rules.len(), 1);
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::json!({"rules":{"<workspace>/input":"readonly"}})
        );
    }

    #[test]
    fn legacy_grants_require_explicit_confirmation_of_recursive_scope() {
        for path in [
            "<workspace>/input",
            "<workspace>/input/**",
            "<workspace>/result.txt",
        ] {
            let error = serde_json::from_value::<PathPermissionConfig>(serde_json::json!({
                "rules": {path: {"read":"allow","write":"deny"}}
            }))
            .unwrap_err()
            .to_string();
            assert!(error.contains("without changing its scope"), "{error}");
        }
        let modes = serde_json::json!({"read":"allow","write":"allow"});
        let value: PathPermissionConfig = serde_json::from_value(serde_json::json!({
            "workspace":{"read":"allow","write":"deny"},
            "rules": {
                "<workspace>/output": modes,
                "<workspace>/output/**": modes
            }
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::json!({"workspace":"readonly","rules":{"<workspace>/output":"readwrite"}})
        );
        let denied: PathPermissionConfig = serde_json::from_value(serde_json::json!({
            "rules":{"<home>/.ssh":{"read":"deny","write":"deny"}}
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(denied).unwrap(),
            serde_json::json!({"rules":{"<home>/.ssh":"deny"}})
        );
    }

    #[test]
    fn concrete_directory_rules_accept_nested_readonly_and_readwrite_modes() {
        for (parent, child) in [("readonly", "readwrite"), ("readwrite", "readonly")] {
            let value = serde_json::json!({"rules": {
                "<workspace>/data": parent,
                "<workspace>/data/output": child,
                "<workspace>/data/output/reference": parent
            }});
            let config: PathPermissionConfig = serde_json::from_value(value.clone()).unwrap();
            config.validate().unwrap();
            assert_eq!(serde_json::to_value(config).unwrap(), value);
        }
        for child in ["readonly", "readwrite"] {
            let error =
                serde_json::from_value::<PathPermissionConfig>(serde_json::json!({"rules": {
                    "<workspace>/secrets": "deny",
                    "<workspace>/secrets/public": child
                }}))
                .unwrap_err()
                .to_string();
            assert!(error.contains("explicitly denied"), "{error}");
        }
    }

    #[test]
    fn explicit_filesystem_root_denials_cannot_be_reopened_by_a_child() {
        for (parent, child) in [("/", "/srv/output"), ("C:/", "C:/work/output")] {
            let error =
                serde_json::from_value::<PathPermissionConfig>(serde_json::json!({"rules": {
                    parent:"deny", child:"readwrite"
                }}))
                .unwrap_err()
                .to_string();
            assert!(error.contains("explicitly denied"), "{error}");
        }
    }

    #[test]
    fn ambiguous_or_unrepresentable_paths_fail_early() {
        for value in [
            "",
            " ",
            " <workspace>/input",
            "<workspace>input",
            "<unknown>/input",
            "<workspace",
            "<home>/../secret",
            "../secret",
            "<workspace>/**/*.key",
            "secret?.txt",
            "<workspace>/input/**/**",
            "~/.ssh",
            "C:",
            "C:relative",
            "//server/share",
            "/tmp/file:stream",
        ] {
            assert!(canonical_permission_path(value).is_err(), "{value}");
        }
        assert!(
            serde_json::from_value::<PathPermissionConfig>(serde_json::json!({
                "rules": {
                    "<workspace>/input": "readonly",
                    "<workspace>/input/**": "readwrite"
                }
            }))
            .is_err()
        );
    }
}
