use std::collections::BTreeMap;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use landlock::ABI;
use landlock::Access;
use landlock::AccessFs;
use landlock::CompatLevel;
use landlock::Compatible;
use landlock::Ruleset;
use landlock::RulesetAttr;
use landlock::RulesetCreatedAttr;
use seccompiler::BpfProgram;
use seccompiler::SeccompAction;
use seccompiler::SeccompCmpArgLen;
use seccompiler::SeccompCmpOp;
use seccompiler::SeccompCondition;
use seccompiler::SeccompFilter;
use seccompiler::SeccompRule;
use seccompiler::TargetArch;
use seccompiler::apply_filter;

use crate::sandbox::{SandboxCommandRequest, SandboxError, SandboxExecOutput, SandboxPolicy};

use super::command_runner::{configure_piped_stdio, run_command_with_timeout};

pub(super) fn execute(
    request: &SandboxCommandRequest,
    policy: &SandboxPolicy,
    workspace_root: &Path,
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

    if !matches!(policy, SandboxPolicy::DangerFullAccess) {
        let writable_roots = policy
            .writable_roots_with_workspace(workspace_root)
            .into_iter()
            .map(|root| root.root)
            .collect::<Vec<_>>();
        let network_access = policy.has_full_network_access();

        unsafe {
            command.pre_exec(move || {
                if !network_access {
                    install_network_seccomp_filter_on_current_thread()?;
                }
                install_filesystem_landlock_rules_on_current_thread(&writable_roots)?;
                Ok(())
            });
        }
    }

    run_command_with_timeout(&mut command, request.timeout_ms, start)
}

fn install_filesystem_landlock_rules_on_current_thread(
    writable_roots: &[PathBuf],
) -> io::Result<()> {
    let abi = ABI::V5;
    let access_rw = AccessFs::from_all(abi);
    let access_ro = AccessFs::from_read(abi);

    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(access_rw)
        .map_err(to_io_error)?
        .create()
        .map_err(to_io_error)?
        .add_rules(landlock::path_beneath_rules(&["/"], access_ro))
        .map_err(to_io_error)?
        .add_rules(landlock::path_beneath_rules(&["/dev/null"], access_rw))
        .map_err(to_io_error)?
        .set_no_new_privs(true);

    if !writable_roots.is_empty() {
        let refs = writable_roots
            .iter()
            .map(PathBuf::as_path)
            .collect::<Vec<_>>();
        ruleset = ruleset
            .add_rules(landlock::path_beneath_rules(&refs, access_rw))
            .map_err(to_io_error)?;
    }

    let status = ruleset.restrict_self().map_err(to_io_error)?;
    if status.ruleset == landlock::RulesetStatus::NotEnforced {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "landlock could not enforce all filesystem rules",
        ));
    }
    Ok(())
}

fn install_network_seccomp_filter_on_current_thread() -> io::Result<()> {
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();

    let mut deny_syscall = |number: i64| {
        rules.insert(number, vec![]);
    };

    deny_syscall(libc::SYS_connect);
    deny_syscall(libc::SYS_accept);
    deny_syscall(libc::SYS_accept4);
    deny_syscall(libc::SYS_bind);
    deny_syscall(libc::SYS_listen);
    deny_syscall(libc::SYS_getpeername);
    deny_syscall(libc::SYS_getsockname);
    deny_syscall(libc::SYS_shutdown);
    deny_syscall(libc::SYS_sendto);
    deny_syscall(libc::SYS_sendmsg);
    deny_syscall(libc::SYS_sendmmsg);
    deny_syscall(libc::SYS_recvmsg);
    deny_syscall(libc::SYS_recvmmsg);
    deny_syscall(libc::SYS_getsockopt);
    deny_syscall(libc::SYS_setsockopt);
    deny_syscall(libc::SYS_ptrace);

    let unix_only = SeccompRule::new(vec![
        SeccompCondition::new(
            0,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::Ne,
            libc::AF_UNIX as u64,
        )
        .map_err(to_io_error)?,
    ])
    .map_err(to_io_error)?;
    rules.insert(libc::SYS_socket, vec![unix_only.clone()]);
    rules.insert(libc::SYS_socketpair, vec![unix_only]);

    let arch = if cfg!(target_arch = "x86_64") {
        TargetArch::x86_64
    } else if cfg!(target_arch = "aarch64") {
        TargetArch::aarch64
    } else {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unsupported architecture for seccomp sandbox",
        ));
    };

    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        arch,
    )
    .map_err(to_io_error)?;

    let program: BpfProgram = filter.try_into().map_err(to_io_error)?;
    apply_filter(&program).map_err(to_io_error)?;
    Ok(())
}

fn to_io_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, error.to_string())
}
