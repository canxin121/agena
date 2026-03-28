mod acl;
mod audit;
mod env;
mod process;
mod token;
mod util;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::sandbox::{
    SandboxCommandRequest, SandboxError, SandboxExecOutput, SandboxPolicy, cap_fs,
};

use super::command_runner::{configure_piped_stdio, run_command_with_timeout};

pub(super) fn execute(
    request: &SandboxCommandRequest,
    policy: &SandboxPolicy,
    workspace_root: &Path,
) -> Result<SandboxExecOutput, SandboxError> {
    if matches!(policy, SandboxPolicy::DangerFullAccess) {
        return execute_without_sandbox(request);
    }

    let start = Instant::now();
    let mut env_map = request.env.clone();
    util::normalize_null_device_env(&mut env_map);
    util::ensure_non_interactive_pager(&mut env_map);

    if !policy.has_full_network_access() {
        env::apply_no_network_hardening(&mut env_map)?;
    }

    let acl_plan = collect_acl_plan(policy, workspace_root)?;
    let allow_paths = sanitize_allow_paths(policy, acl_plan.allow_paths)?;
    let deny_paths = sanitize_allow_paths(policy, acl_plan.deny_paths)?;

    if policy.enforce_world_writable_audit() {
        audit::audit_paths_for_world_writable(&allow_paths, &env_map, &request.cwd)?;
    }

    let executable = process::resolve_executable(&request.command[0], &request.cwd, &env_map)
        .ok_or_else(|| {
            SandboxError::InvalidRequest(format!(
                "unable to resolve executable '{}' from cwd {}",
                request.command[0],
                request.cwd.display()
            ))
        })?;

    let capability_sid_string = token::random_capability_sid();
    let capability_sid = token::OwnedSid::from_string_sid(&capability_sid_string)?;
    let restricted_token = token::create_restricted_token_with_capability(capability_sid.raw())?;

    let mut acl_rollback = acl::AclRollback::new(capability_sid.raw());
    for path in &deny_paths {
        let added = unsafe { acl::add_deny_write_ace(path, capability_sid.raw())? };
        if added {
            acl_rollback.track(path.clone());
        }
    }
    for path in &allow_paths {
        let added = unsafe { acl::add_allow_ace(path, capability_sid.raw())? };
        if added {
            acl_rollback.track(path.clone());
        }
    }
    unsafe {
        acl::allow_null_device(capability_sid.raw());
    }

    let capture = process::run_process_as_user(
        restricted_token.raw(),
        &executable,
        &request.command,
        &request.cwd,
        &env_map,
        request.timeout_ms,
    )?;

    drop(acl_rollback);

    let stdout = String::from_utf8_lossy(&capture.stdout).to_string();
    let stderr = String::from_utf8_lossy(&capture.stderr).to_string();

    Ok(SandboxExecOutput {
        exit_code: capture.exit_code,
        stdout: stdout.clone(),
        stderr: stderr.clone(),
        aggregated_output: format!("{stdout}{stderr}"),
        duration: start.elapsed(),
        timed_out: capture.timed_out,
    })
}

struct AclPlan {
    allow_paths: Vec<PathBuf>,
    deny_paths: Vec<PathBuf>,
}

fn collect_acl_plan(
    policy: &SandboxPolicy,
    workspace_root: &Path,
) -> Result<AclPlan, SandboxError> {
    if !matches!(policy, SandboxPolicy::WorkspaceWrite { .. }) {
        return Ok(AclPlan {
            allow_paths: Vec::new(),
            deny_paths: Vec::new(),
        });
    }

    let writable_roots = policy.writable_roots_with_workspace(workspace_root);
    if writable_roots.is_empty() {
        return Err(SandboxError::InvalidRequest(
            "workspace-write sandbox has no writable roots".to_string(),
        ));
    }

    let mut allow_paths = Vec::new();
    let mut deny_paths = Vec::new();
    for writable_root in writable_roots {
        allow_paths.push(writable_root.root);
        deny_paths.extend(writable_root.read_only_subpaths);
    }

    Ok(AclPlan {
        allow_paths,
        deny_paths,
    })
}

fn sanitize_allow_paths(
    policy: &SandboxPolicy,
    allow_paths: Vec<PathBuf>,
) -> Result<Vec<PathBuf>, SandboxError> {
    cap_fs::PathPolicy::ascii_case_insensitive().validate_and_dedupe(allow_paths, |path| {
        util::ensure_safe_allow_path(path, policy.reject_reparse_points())
    })
}

fn execute_without_sandbox(
    request: &SandboxCommandRequest,
) -> Result<SandboxExecOutput, SandboxError> {
    let start = Instant::now();

    let mut command = Command::new(&request.command[0]);
    if request.command.len() > 1 {
        command.args(&request.command[1..]);
    }

    command
        .current_dir(&request.cwd)
        .env_clear()
        .envs(request.env.clone());
    configure_piped_stdio(&mut command);

    run_command_with_timeout(&mut command, request.timeout_ms, start)
}
