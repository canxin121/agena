use std::sync::LazyLock;
use std::{collections::HashMap, path::PathBuf};

use agena_domain::FilesystemEffects;
use agena_process::shell::ShellKind;
use agena_process::shell::ShellPreference;
use agena_process::shell::ShellSettings;
use agena_process::shell::SnapshotMode;
use agena_process::shell::snapshot;
use agena_process::shell::snapshot::ShellSnapshot;
use agena_tool::shell::ShellDialect;
use agena_tool::shell::ShellLaunchSpec;
pub(crate) use agena_tool::shell_analysis::{ExitInterpretation, analyze_command};

use super::{ToolError, ToolExecutor};

/// Trusted shell configuration for this process tree; the environment is read
/// once and never re-read, matching the sandbox-policy surface.
static SHELL_SETTINGS: LazyLock<ShellSettings> = LazyLock::new(ShellSettings::from_env);

pub(crate) fn shell_settings() -> &'static ShellSettings {
    &SHELL_SETTINGS
}

/// Dialect whose flag shape a detected shell expects.
pub(crate) fn shell_dialect(kind: ShellKind) -> ShellDialect {
    match kind {
        ShellKind::Bash => ShellDialect::Bash,
        ShellKind::Zsh => ShellDialect::Zsh,
        ShellKind::Sh => ShellDialect::Sh,
        ShellKind::Powershell => ShellDialect::Powershell,
        ShellKind::Cmd => ShellDialect::Cmd,
    }
}

/// Resolve the launch for a POSIX-family command and fold the interactive
/// startup snapshot into `env`.
///
/// `env` must still be the inherited environment when this is called: snapshot
/// values are inserted here so the plugin `shell.env` overrides applied
/// afterwards win, which is the documented precedence
/// (inherited < snapshot < plugin overrides).
pub(crate) async fn prepare_posix_launch(
    settings: &ShellSettings,
    env: &mut HashMap<String, String>,
) -> Result<ShellLaunchSpec, ToolError> {
    let detected = settings.resolve(ShellPreference::Posix);
    let mut spec = ShellLaunchSpec {
        program: detected.path.to_string_lossy().to_string(),
        dialect: shell_dialect(detected.kind),
        login: settings.login(),
        preamble: String::new(),
        source: detected.source.as_str().to_string(),
        snapshot: SnapshotMode::Disabled.as_str().to_string(),
    };
    if !detected.kind.supports_snapshot() {
        spec.snapshot = "unsupported".to_string();
        return Ok(spec);
    }
    let mode = settings.snapshot();
    if mode == SnapshotMode::Disabled {
        return Ok(spec);
    }
    match snapshot::cache()
        .get_or_capture(detected.kind, &detected.path, env)
        .await
    {
        Ok(captured) => {
            let keys = apply_snapshot(env, &captured);
            spec.preamble = captured.replay_preamble();
            // The capture already ran the login startup, so the command itself
            // must not run it a second time.
            spec.login = false;
            spec.snapshot = format!("used(env={keys},dropped={})", captured.dropped_records);
            Ok(spec)
        }
        Err(error) => {
            if mode == SnapshotMode::Required {
                return Err(ToolError::invalid_input(format!(
                    "shell startup snapshot is required but failed: {}",
                    error.as_str()
                )));
            }
            spec.snapshot = format!("fallback: {}", error.as_str());
            Ok(spec)
        }
    }
}

/// Insert the snapshot's environment values; the snapshot's own filter already
/// dropped Agena control variables, linker variables, and startup injections.
fn apply_snapshot(env: &mut HashMap<String, String>, captured: &ShellSnapshot) -> usize {
    for (key, value) in &captured.env {
        env.insert(key.clone(), value.clone());
    }
    captured.env.len()
}

pub(crate) fn inherited_environment() -> HashMap<String, String> {
    std::env::vars().collect::<HashMap<_, _>>()
}

pub(crate) fn resolve_workdir(
    executor: &ToolExecutor,
    workdir: Option<&str>,
) -> Result<PathBuf, ToolError> {
    let cwd = workdir
        .map(|value| executor.resolve_target_path(value))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());
    Ok(cwd)
}

pub(crate) fn validate_declared_filesystem_effects(
    tool_name: &str,
    command: &str,
    effects: &FilesystemEffects,
) -> Result<(), ToolError> {
    // Only require declarations when the command provably mutates or reads
    // explicit files (write/redirect, input redirect, curl file ops).
    // Interpreters and build tools (node, python, uv, cargo, ...) may run with
    // an explicit empty list: their executable paths are not file effects.
    if effects.is_empty()
        && let Some(reason) =
            agena_tool::shell_analysis::filesystem_effects_required_reason(command)
    {
        return Err(ToolError::invalid_input(format!(
            "{tool_name} must declare every accessed path in reads/writes because the command provably mutates or reads the filesystem: {reason}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_detected_kind_maps_to_its_dialect() {
        assert_eq!(shell_dialect(ShellKind::Bash), ShellDialect::Bash);
        assert_eq!(shell_dialect(ShellKind::Zsh), ShellDialect::Zsh);
        assert_eq!(shell_dialect(ShellKind::Sh), ShellDialect::Sh);
        assert_eq!(
            shell_dialect(ShellKind::Powershell),
            ShellDialect::Powershell
        );
        assert_eq!(shell_dialect(ShellKind::Cmd), ShellDialect::Cmd);
    }

    fn settings_from(pairs: &[(&str, &str)]) -> ShellSettings {
        let pairs = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect::<HashMap<_, _>>();
        ShellSettings::from_lookup(&move |key: &str| pairs.get(key).cloned())
    }

    #[tokio::test]
    async fn disabled_snapshot_keeps_the_login_startup_and_reports_provenance() {
        let settings = settings_from(&[(agena_process::shell::SHELL_SNAPSHOT_ENV, "disabled")]);
        let mut env = HashMap::new();
        let spec = prepare_posix_launch(&settings, &mut env)
            .await
            .expect("launch resolves");
        assert!(!spec.program.is_empty());
        assert!(spec.login);
        assert!(spec.preamble.is_empty());
        assert!(
            env.is_empty(),
            "a disabled snapshot must not touch the environment"
        );
        assert!(matches!(spec.snapshot.as_str(), "disabled" | "unsupported"));
        let metadata = spec.metadata();
        assert!(metadata.iter().any(|(key, _)| key == "shell_program"));
        assert!(metadata.iter().any(|(key, _)| key == "shell_snapshot"));
    }

    #[tokio::test]
    async fn login_startup_can_be_disabled_without_a_snapshot() {
        let settings = settings_from(&[
            (agena_process::shell::SHELL_SNAPSHOT_ENV, "disabled"),
            (agena_process::shell::SHELL_LOGIN_ENV, "0"),
        ]);
        let mut env = HashMap::new();
        let spec = prepare_posix_launch(&settings, &mut env)
            .await
            .expect("launch resolves");
        if agena_process::shell::ShellKind::from_path(std::path::Path::new(&spec.program))
            .is_some_and(|kind| kind.supports_snapshot())
        {
            assert!(!spec.login, "POSIX shells must honor AGENA_SHELL_LOGIN=0");
        }
    }
}
