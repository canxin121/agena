use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::sandbox::{SandboxError, cap_fs};

const DENY_BIN_DIR_NAME: &str = "agena-sandbox-denybin";

pub(super) fn apply_no_network_hardening(
    env_map: &mut HashMap<String, String>,
) -> Result<(), SandboxError> {
    apply_no_network_hardening_with_denybin_dir(env_map, None)
}

fn apply_no_network_hardening_with_denybin_dir(
    env_map: &mut HashMap<String, String>,
    denybin_dir: Option<&Path>,
) -> Result<(), SandboxError> {
    const PROXY_BLACKHOLE: &str = "http://127.0.0.1:9";
    const LOCAL_NO_PROXY: &str = "localhost,127.0.0.1,::1";

    set_case_insensitive(env_map, "AGENA_SANDBOX_NETWORK_DISABLED", "1");

    // Fail-closed: overwrite existing proxy-related settings (including any
    // mixed-case variants that might bypass simple exact-key updates).
    set_case_insensitive_pair(env_map, "HTTP_PROXY", "http_proxy", PROXY_BLACKHOLE);
    set_case_insensitive_pair(env_map, "HTTPS_PROXY", "https_proxy", PROXY_BLACKHOLE);
    set_case_insensitive_pair(env_map, "ALL_PROXY", "all_proxy", PROXY_BLACKHOLE);
    set_case_insensitive_pair(env_map, "GIT_HTTP_PROXY", "git_http_proxy", PROXY_BLACKHOLE);
    set_case_insensitive_pair(
        env_map,
        "GIT_HTTPS_PROXY",
        "git_https_proxy",
        PROXY_BLACKHOLE,
    );

    set_case_insensitive_pair(env_map, "NO_PROXY", "no_proxy", LOCAL_NO_PROXY);
    set_case_insensitive(env_map, "PIP_NO_INDEX", "1");
    set_case_insensitive(env_map, "PIP_DISABLE_PIP_VERSION_CHECK", "1");
    set_case_insensitive(env_map, "NPM_CONFIG_OFFLINE", "true");
    set_case_insensitive(env_map, "CARGO_NET_OFFLINE", "true");
    set_case_insensitive(env_map, "GIT_SSH_COMMAND", "cmd /c exit 1");
    set_case_insensitive(env_map, "GIT_ALLOW_PROTOCOL", "");
    set_case_insensitive(env_map, "GIT_ALLOW_PROTOCOLS", "");
    set_case_insensitive(env_map, "GIT_PROTOCOL_FROM_USER", "0");
    set_case_insensitive(env_map, "GIT_TERMINAL_PROMPT", "0");

    let deny_bin = ensure_denybin(
        &["ssh", "scp", "sftp", "ftp", "telnet", "nc", "ncat"],
        denybin_dir,
    )?;
    prepend_path(env_map, &deny_bin.to_string_lossy());
    reorder_pathext_for_stubs(env_map);
    Ok(())
}

fn ensure_denybin(tools: &[&str], custom_dir: Option<&Path>) -> Result<PathBuf, SandboxError> {
    let base = match custom_dir {
        Some(path) => path.to_path_buf(),
        None => env::temp_dir().join(DENY_BIN_DIR_NAME),
    };
    fs::create_dir_all(&base)?;

    for tool in tools {
        for ext in [".bat", ".cmd"] {
            let path = base.join(format!("{tool}{ext}"));
            if cap_fs::path_exists(&path) {
                continue;
            }
            let mut file = File::create(&path)?;
            file.write_all(b"@echo off\r\nexit /b 1\r\n")?;
        }
    }
    Ok(base)
}

fn prepend_path(env_map: &mut HashMap<String, String>, prefix: &str) {
    let existing = get_env_case_insensitive(env_map, "PATH")
        .or_else(|| env::var("PATH").ok())
        .unwrap_or_default();
    let first = existing.split(';').next().unwrap_or_default();
    if first.eq_ignore_ascii_case(prefix) {
        remove_case_insensitive_entries(env_map, "PATH");
        env_map.insert("PATH".to_string(), existing);
        return;
    }

    let new_path = if existing.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix};{existing}")
    };
    remove_case_insensitive_entries(env_map, "PATH");
    env_map.insert("PATH".to_string(), new_path);
}

fn reorder_pathext_for_stubs(env_map: &mut HashMap<String, String>) {
    let default = get_env_case_insensitive(env_map, "PATHEXT")
        .or_else(|| env::var("PATHEXT").ok())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());

    let extensions = default
        .split(';')
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>();
    let upper = extensions
        .iter()
        .map(|entry| entry.to_ascii_uppercase())
        .collect::<Vec<_>>();

    let mut ordered = Vec::new();
    for wanted in [".BAT", ".CMD"] {
        if let Some(index) = upper.iter().position(|entry| entry == wanted) {
            ordered.push(extensions[index].clone());
        }
    }
    for (index, ext) in extensions.into_iter().enumerate() {
        let up = &upper[index];
        if up == ".BAT" || up == ".CMD" {
            continue;
        }
        ordered.push(ext);
    }

    remove_case_insensitive_entries(env_map, "PATHEXT");
    env_map.insert("PATHEXT".to_string(), ordered.join(";"));
}

fn get_env_case_insensitive(env_map: &HashMap<String, String>, key: &str) -> Option<String> {
    env_map.get(key).cloned().or_else(|| {
        env_map
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.clone())
    })
}

fn remove_case_insensitive_entries(env_map: &mut HashMap<String, String>, key: &str) {
    let to_remove = env_map
        .keys()
        .filter(|candidate| candidate.eq_ignore_ascii_case(key))
        .cloned()
        .collect::<Vec<_>>();
    for existing in to_remove {
        env_map.remove(&existing);
    }
}

fn set_case_insensitive(env_map: &mut HashMap<String, String>, key: &str, value: &str) {
    remove_case_insensitive_entries(env_map, key);
    env_map.insert(key.to_string(), value.to_string());
}

fn set_case_insensitive_pair(
    env_map: &mut HashMap<String, String>,
    upper_key: &str,
    lower_key: &str,
    value: &str,
) {
    remove_case_insensitive_entries(env_map, upper_key);
    env_map.insert(upper_key.to_string(), value.to_string());
    env_map.insert(lower_key.to_string(), value.to_string());
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        apply_no_network_hardening_with_denybin_dir, prepend_path, reorder_pathext_for_stubs,
    };

    #[test]
    fn hardening_overwrites_mixed_case_proxy_and_git_guards() {
        let mut env = std::collections::HashMap::new();
        env.insert("Http_Proxy".to_string(), "http://attacker:8080".to_string());
        env.insert("NO_proxy".to_string(), "*".to_string());
        env.insert("git_allow_protocol".to_string(), "ssh".to_string());
        env.insert("GIT_PROTOCOL_FROM_USER".to_string(), "1".to_string());
        env.insert("Path".to_string(), r"C:\\Windows\\System32".to_string());
        env.insert("PATHEXT".to_string(), ".EXE;.CMD;.BAT".to_string());

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let denybin_dir = std::env::temp_dir().join(format!("agena-sandbox-denybin-test-{nonce}"));

        apply_no_network_hardening_with_denybin_dir(&mut env, Some(&denybin_dir))
            .expect("network hardening should succeed");

        assert_eq!(
            env.get("HTTP_PROXY"),
            Some(&"http://127.0.0.1:9".to_string())
        );
        assert_eq!(
            env.get("http_proxy"),
            Some(&"http://127.0.0.1:9".to_string())
        );
        assert_eq!(
            env.get("NO_PROXY"),
            Some(&"localhost,127.0.0.1,::1".to_string())
        );
        assert_eq!(
            env.get("no_proxy"),
            Some(&"localhost,127.0.0.1,::1".to_string())
        );
        assert_eq!(env.get("GIT_ALLOW_PROTOCOL"), Some(&"".to_string()));
        assert_eq!(env.get("GIT_ALLOW_PROTOCOLS"), Some(&"".to_string()));
        assert_eq!(env.get("GIT_PROTOCOL_FROM_USER"), Some(&"0".to_string()));
        assert_eq!(env.get("GIT_TERMINAL_PROMPT"), Some(&"0".to_string()));

        assert!(!env.contains_key("Http_Proxy"));
        assert!(!env.contains_key("NO_proxy"));
        assert!(!env.contains_key("git_allow_protocol"));
        assert!(!env.contains_key("Path"));

        let path = env.get("PATH").expect("PATH should be present");
        assert!(path.starts_with(&denybin_dir.to_string_lossy().to_string()));
        assert_eq!(env.get("PATHEXT"), Some(&".BAT;.CMD;.EXE".to_string()));

        let _ = std::fs::remove_dir_all(&denybin_dir);
    }

    #[test]
    fn prepend_path_keeps_existing_head_prefix_case_insensitively() {
        let mut env = std::collections::HashMap::new();
        env.insert(
            "Path".to_string(),
            r"C:\\sandbox-denybin;C:\\Windows\\System32".to_string(),
        );

        prepend_path(&mut env, r"c:\\SANDBOX-denybin");

        assert_eq!(
            env.get("PATH"),
            Some(&r"C:\\sandbox-denybin;C:\\Windows\\System32".to_string())
        );
        assert!(!env.contains_key("Path"));
    }

    #[test]
    fn reorder_pathext_moves_bat_and_cmd_to_front() {
        let mut env = std::collections::HashMap::new();
        env.insert("PathExt".to_string(), ".EXE;.COM;.CMD;.BAT".to_string());

        reorder_pathext_for_stubs(&mut env);

        assert_eq!(env.get("PATHEXT"), Some(&".BAT;.CMD;.EXE;.COM".to_string()));
        assert!(!env.contains_key("PathExt"));
    }
}
