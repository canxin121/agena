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
static SHELL_DETECTION: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

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
    let owned_settings = settings.clone();
    let detected = SHELL_DETECTION
        .run(move || owned_settings.resolve(ShellPreference::Posix))
        .await
        .map_err(|error| {
            std::io::Error::other(format!("shell detection worker failed: {error}"))
        })?;
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

/// Shell working directory with the caller's base folder as the last resort.
///
/// A project session keeps requiring nothing special: its project root is the
/// default. A global (Bot) session has no root to run in, so it must name
/// either an absolute `workdir` or an absolute `folder`; omitted both stays an
/// explicit error instead of silently running somewhere meaningless.
pub(crate) fn resolve_workdir_in_folder(
    executor: &ToolExecutor,
    workdir: Option<&str>,
    folder: Option<&str>,
) -> Result<PathBuf, ToolError> {
    let usable = |path: &str| !path.trim().is_empty() && std::path::Path::new(path).is_absolute();
    if executor.is_global() && !workdir.is_some_and(usable) && !folder.is_some_and(usable) {
        return Err(ToolError::invalid_input(
            "global shell execution requires an explicit absolute workdir or folder",
        ));
    }
    let cwd = match workdir {
        Some(value) => executor.resolve_target_path_in_folder(value, folder),
        None => executor.resolve_folder_base(folder),
    };
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
