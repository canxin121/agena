//! Runtime-PATH capability discovery. Default inspection only stats files;
//! explicit version probes use managed children with bounded output/time.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agena_tool::cli_catalog::{CLI_CATALOG, CliToolSpec};
use serde::Serialize;

static WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
static CACHE: Mutex<Option<CachedInventory>> = Mutex::new(None);
const CACHE_TTL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchEnvironment {
    workspace: PathBuf,
    path: Option<OsString>,
    path_ext: OsString,
}

struct CachedInventory {
    environment: SearchEnvironment,
    checked: Instant,
    inventory: CliInventory,
}

#[derive(Debug, Clone, Serialize)]
pub struct CliAvailability {
    pub name: String,
    pub purpose: String,
    pub tier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
    pub available: bool,
    pub guidance: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CliInventory {
    pub tools: Vec<CliAvailability>,
    pub checked_at_unix_ms: u64,
    pub cache_age_ms: u64,
}

impl CliInventory {
    pub fn available_names(&self) -> Vec<&str> {
        self.tools
            .iter()
            .filter(|tool| tool.available)
            .map(|tool| tool.name.as_str())
            .collect()
    }

    /// Small environment payload; detailed guidance is queried only on demand.
    pub fn compact(&self) -> serde_json::Value {
        serde_json::json!({
            "available": self.tools.iter().filter(|tool| tool.available)
                .map(|tool| (tool.name.as_str(), &tool.executable)).collect::<std::collections::BTreeMap<_, _>>(),
            "checked_at_unix_ms": self.checked_at_unix_ms,
            "cache_age_ms": self.cache_age_ms,
            "versions_probed": false,
        })
    }
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        // Check this process's actual access, rather than accepting any x bit.
        unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn executable_names(name: &str, path_ext: &std::ffi::OsStr, windows: bool) -> Vec<OsString> {
    let mut names = vec![OsString::from(name)];
    if windows {
        let extensions = path_ext
            .to_str()
            .filter(|value| !value.is_empty())
            .unwrap_or(".COM;.EXE;.BAT;.CMD");
        for extension in extensions
            .split(';')
            .filter(|extension| extension.starts_with('.') && !extension.contains(['/', '\\']))
        {
            names.push(OsString::from(format!("{name}{extension}")));
        }
    }
    names
}

fn resolve(spec: &CliToolSpec, environment: &SearchEnvironment) -> Option<PathBuf> {
    let path = environment.path.as_ref()?;
    for name in spec.executables {
        let names = executable_names(name, &environment.path_ext, cfg!(windows));
        for directory in std::env::split_paths(path) {
            let directory = if directory.is_absolute() {
                directory
            } else {
                environment.workspace.join(directory)
            };
            for name in &names {
                let candidate = directory.join(name);
                if executable(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn detect(environment: &SearchEnvironment) -> CliInventory {
    let tools = CLI_CATALOG
        .iter()
        .map(|spec| {
            let executable = resolve(spec, environment);
            CliAvailability {
                name: spec.name.into(),
                purpose: spec.purpose.into(),
                tier: spec.tier.into(),
                available: executable.is_some(),
                executable,
                guidance: spec.guidance.into(),
                version: None,
                probe_error: None,
            }
        })
        .collect();
    CliInventory {
        tools,
        checked_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64,
        cache_age_ms: 0,
    }
}

fn cached_detect(environment: SearchEnvironment, refresh: bool) -> CliInventory {
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !refresh
        && let Some(cached) = cache.as_ref()
        && cached.environment == environment
        && cached.checked.elapsed() < CACHE_TTL
        && cached
            .inventory
            .tools
            .iter()
            .filter_map(|tool| tool.executable.as_deref())
            .all(executable)
    {
        let mut inventory = cached.inventory.clone();
        inventory.cache_age_ms = cached.checked.elapsed().as_millis() as u64;
        return inventory;
    }
    let inventory = detect(&environment);
    *cache = Some(CachedInventory {
        environment,
        checked: Instant::now(),
        inventory: inventory.clone(),
    });
    inventory
}

/// Probe in the same PATH and workspace used by the runtime, never the user's
/// interactive shell startup files. Explicit refresh also notices new installs.
/// An unset PATH reports no candidates; it never implies the workspace or
/// guesses a platform/shell-specific fallback path.
pub async fn discover(workspace: &Path, refresh: bool) -> std::io::Result<CliInventory> {
    let environment = SearchEnvironment {
        workspace: workspace.to_path_buf(),
        path: std::env::var_os("PATH"),
        path_ext: std::env::var_os("PATHEXT").unwrap_or_default(),
    };
    let permit = WORKERS
        .acquire()
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        cached_detect(environment, refresh)
    })
    .await
    .map_err(|error| std::io::Error::other(error.to_string()))
}

/// Explicit probes only; callers bound the number of selected tools. No shell,
/// package installation, model download or arbitrary arguments are involved.
pub async fn probe_version(tool: &mut CliAvailability, workspace: &Path) {
    tool.version = None;
    tool.probe_error = None;
    let Some(path) = tool.executable.as_deref() else {
        return;
    };
    let Some(spec) = agena_tool::cli_catalog::find(&tool.name) else {
        return;
    };
    let _permit = match WORKERS.acquire().await {
        Ok(permit) => permit,
        Err(error) => {
            tool.probe_error = Some(error.to_string());
            return;
        }
    };
    let mut command = tokio::process::Command::new(path);
    command
        .args(spec.version_args)
        .current_dir(workspace)
        .stdin(std::process::Stdio::null())
        .env("NO_COLOR", "1")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat");
    match agena_process::output(command, Duration::from_secs(2), 8192).await {
        Ok(output) if output.status.success() => {
            let bytes = if output.stdout.is_empty() {
                &output.stderr
            } else {
                &output.stdout
            };
            let text = String::from_utf8_lossy(bytes);
            tool.version = text
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(|line| {
                    line.chars()
                        .filter(|ch| !ch.is_control())
                        .take(240)
                        .collect::<String>()
                });
            if tool.version.is_none() {
                tool.probe_error = Some("Version command returned no version text".into());
            }
        }
        Ok(output) => {
            tool.probe_error = Some(format!("Version command exited with {}", output.status))
        }
        Err(error) => tool.probe_error = Some(format!("Version probe failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_extensions_are_explicit_and_do_not_accept_paths() {
        let names = executable_names("rg", std::ffi::OsStr::new(".EXE;.CMD;../bad"), true);
        assert_eq!(names, ["rg", "rg.EXE", "rg.CMD"]);
    }

    #[cfg(unix)]
    fn script(path: &Path, contents: &str, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, contents).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn discovers_real_executables_and_aliases_without_running_them() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        script(
            &bin.join("rg"),
            "#!/bin/sh\ntouch SHOULD_NOT_EXIST\n",
            0o755,
        );
        script(&bin.join("fdfind"), "#!/bin/sh\nexit 0\n", 0o755);
        script(&bin.join("jq"), "#!/bin/sh\nexit 0\n", 0o644);
        std::fs::create_dir(bin.join("uv")).unwrap();
        let environment = SearchEnvironment {
            workspace: root.path().into(),
            path: Some("bin".into()),
            path_ext: OsString::new(),
        };
        let inventory = detect(&environment);
        assert!(inventory.available_names().contains(&"rg"));
        let fd = inventory
            .tools
            .iter()
            .find(|tool| tool.name == "fd")
            .unwrap();
        assert_eq!(fd.executable.as_deref(), Some(bin.join("fdfind").as_path()));
        assert!(!inventory.available_names().contains(&"jq"));
        assert!(!inventory.available_names().contains(&"uv"));
        assert!(!root.path().join("SHOULD_NOT_EXIST").exists());
        assert!(inventory.tools.iter().all(|tool| tool.version.is_none()));
    }

    #[cfg(unix)]
    #[test]
    fn cache_refresh_path_changes_and_removed_executables_are_observed() {
        let root = tempfile::tempdir().unwrap();
        let environment = SearchEnvironment {
            workspace: root.path().into(),
            path: Some(root.path().as_os_str().into()),
            path_ext: OsString::new(),
        };
        assert!(
            !cached_detect(environment.clone(), true)
                .available_names()
                .contains(&"rg")
        );
        script(&root.path().join("rg"), "#!/bin/sh\nexit 0\n", 0o755);
        assert!(
            cached_detect(environment.clone(), true)
                .available_names()
                .contains(&"rg")
        );
        std::fs::remove_file(root.path().join("rg")).unwrap();
        assert!(
            !cached_detect(environment.clone(), false)
                .available_names()
                .contains(&"rg")
        );
        let other = SearchEnvironment {
            path: Some(root.path().join("missing").into_os_string()),
            ..environment
        };
        assert!(cached_detect(other, false).available_names().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn unset_path_does_not_implicitly_discover_workspace_programs() {
        let root = tempfile::tempdir().unwrap();
        script(&root.path().join("rg"), "#!/bin/sh\nexit 0\n", 0o755);
        let mut environment = SearchEnvironment {
            workspace: root.path().into(),
            path: None,
            path_ext: OsString::new(),
        };
        assert!(detect(&environment).available_names().is_empty());
        environment.path = Some(OsString::new());
        assert!(detect(&environment).available_names().contains(&"rg"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_probe_handles_stderr_and_nonzero_status() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pdftotext");
        script(
            &path,
            "#!/bin/sh\n[ \"$1\" = -v ] || exit 5\nprintf 'pdftotext fixture 1\\n' >&2\n",
            0o755,
        );
        let environment = SearchEnvironment {
            workspace: root.path().into(),
            path: Some(root.path().as_os_str().into()),
            path_ext: OsString::new(),
        };
        let mut inventory = detect(&environment);
        let tool = inventory
            .tools
            .iter_mut()
            .find(|tool| tool.name == "pdftotext")
            .unwrap();
        probe_version(tool, root.path()).await;
        assert_eq!(tool.version.as_deref(), Some("pdftotext fixture 1"));
        script(&path, "#!/bin/sh\nexit 7\n", 0o755);
        tool.version = None;
        probe_version(tool, root.path()).await;
        assert!(tool.probe_error.as_deref().unwrap().contains('7'));
        assert!(tool.version.is_none());
    }
}
