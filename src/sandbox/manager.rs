use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{SandboxError, SandboxExecOutput, SandboxPolicy, cap_fs, platform};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxCommandRequest {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub timeout_ms: Option<u64>,
}

impl SandboxCommandRequest {
    pub fn validate(&self) -> Result<(), SandboxError> {
        if self.command.is_empty() {
            return Err(SandboxError::InvalidRequest(
                "command must contain at least one token".to_string(),
            ));
        }
        if self.command[0].trim().is_empty() {
            return Err(SandboxError::InvalidRequest(
                "command executable must not be empty".to_string(),
            ));
        }
        if !cap_fs::path_exists(&self.cwd) {
            return Err(SandboxError::InvalidRequest(format!(
                "sandbox cwd does not exist: {}",
                self.cwd.display()
            )));
        }
        if !cap_fs::is_dir(&self.cwd) {
            return Err(SandboxError::InvalidRequest(format!(
                "sandbox cwd is not a directory: {}",
                self.cwd.display()
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct SandboxManager;

impl SandboxManager {
    pub fn new() -> Self {
        Self
    }

    pub fn execute(
        &self,
        request: &SandboxCommandRequest,
        policy: &SandboxPolicy,
        workspace_root: &Path,
    ) -> Result<SandboxExecOutput, SandboxError> {
        request.validate()?;
        let sanitized = sanitize_request_env(request, policy);
        platform::execute(&sanitized, policy, workspace_root)
    }
}

fn sanitize_request_env(
    request: &SandboxCommandRequest,
    policy: &SandboxPolicy,
) -> SandboxCommandRequest {
    let env = sanitize_env_vars(&request.env, policy);
    SandboxCommandRequest {
        command: request.command.clone(),
        cwd: request.cwd.clone(),
        env,
        timeout_ms: request.timeout_ms,
    }
}

fn sanitize_env_vars(
    env: &HashMap<String, String>,
    policy: &SandboxPolicy,
) -> HashMap<String, String> {
    if matches!(policy, SandboxPolicy::DangerFullAccess) {
        return env.clone();
    }

    const BLOCKED_EXACT: [&str; 5] = [
        "BASH_ENV",
        "ENV",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "LD_AUDIT",
    ];
    const BLOCKED_PREFIXES: [&str; 3] = ["DYLD_", "LD_", "BASH_FUNC_"];

    env.iter()
        .filter(|(key, _)| {
            !BLOCKED_EXACT
                .iter()
                .any(|name| key.eq_ignore_ascii_case(name))
                && !BLOCKED_PREFIXES
                    .iter()
                    .any(|prefix| starts_with_ascii_case_insensitive(key, prefix))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn starts_with_ascii_case_insensitive(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::sandbox::SandboxPolicy;

    use super::sanitize_env_vars;

    #[test]
    fn strips_loader_and_shell_injection_env_when_sandboxed() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        env.insert("LD_PRELOAD".to_string(), "evil.so".to_string());
        env.insert(
            "DYLD_INSERT_LIBRARIES".to_string(),
            "evil.dylib".to_string(),
        );
        env.insert("BASH_ENV".to_string(), "/tmp/rc".to_string());

        let sanitized = sanitize_env_vars(&env, &SandboxPolicy::new_workspace_write_policy());

        assert!(sanitized.contains_key("PATH"));
        assert!(!sanitized.contains_key("LD_PRELOAD"));
        assert!(!sanitized.contains_key("DYLD_INSERT_LIBRARIES"));
        assert!(!sanitized.contains_key("BASH_ENV"));
    }

    #[test]
    fn keeps_env_unchanged_for_danger_full_access() {
        let mut env = HashMap::new();
        env.insert("LD_PRELOAD".to_string(), "allowed.so".to_string());

        let sanitized = sanitize_env_vars(&env, &SandboxPolicy::DangerFullAccess);

        assert_eq!(sanitized.get("LD_PRELOAD"), Some(&"allowed.so".to_string()));
    }

    #[test]
    fn strips_blocked_prefixes_case_insensitively() {
        let mut env = HashMap::new();
        env.insert(
            "dyld_insert_libraries".to_string(),
            "evil.dylib".to_string(),
        );
        env.insert("Ld_PreLoAd".to_string(), "evil.so".to_string());
        env.insert("BaSh_FuNc_x".to_string(), "() { :; }".to_string());
        env.insert("SAFE_VAR".to_string(), "1".to_string());

        let sanitized = sanitize_env_vars(&env, &SandboxPolicy::new_read_only_policy());

        assert!(!sanitized.contains_key("dyld_insert_libraries"));
        assert!(!sanitized.contains_key("Ld_PreLoAd"));
        assert!(!sanitized.contains_key("BaSh_FuNc_x"));
        assert_eq!(sanitized.get("SAFE_VAR"), Some(&"1".to_string()));
    }
}
