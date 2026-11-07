//! Linux-only recovery of test-owned descendants that create independent Unix
//! sessions (notably PTYs). Never signal a PID just because it belongs to a
//! familiar process group: only processes observed below the owned product
//! leader and with an unchanged /proc start timestamp are eligible.
//!
//! This is a best-effort ownership fence, not a cgroup/pid-namespace sandbox:
//! a child that daemonizes before the initial snapshot can escape ancestry.

use std::{
    collections::{HashMap, HashSet},
    fs, io,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
struct Identity {
    pid: i32,
    parent: i32,
    started: u64,
    state: char,
    depth: u32,
}

fn identity(pid: i32) -> io::Result<Option<Identity>> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    // The comm field is parenthesized and may itself contain spaces and ')'.
    let fields = stat
        .rsplit_once(") ")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed /proc stat"))?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    let parse = |index: usize| -> io::Result<&str> {
        fields
            .get(index)
            .copied()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "short /proc stat"))
    };
    let state = parse(0)?
        .chars()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing process state"))?;
    let parent = parse(1)?
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    // starttime is field 22, or index 19 after the PID and comm fields.
    let started = parse(19)?
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(Identity {
        pid,
        parent,
        started,
        state,
        depth: 0,
    }))
}

fn still_running(id: &Identity) -> io::Result<bool> {
    Ok(identity(id.pid)?.is_some_and(|current| {
        current.started == id.started && !matches!(current.state, 'Z' | 'X')
    }))
}

#[derive(Clone, Default)]
pub(super) struct Descendants(Vec<Identity>);

impl Descendants {
    /// A tmux pane belongs to this case's private tmux socket, even though
    /// its shell is no longer descended from the Agena process leader.
    pub(super) fn capture_including_root(leader_pid: u32) -> io::Result<Self> {
        let mut children = Self::capture(leader_pid)?;
        if let Some(root) = i32::try_from(leader_pid)
            .ok()
            .and_then(|pid| identity(pid).ok().flatten())
        {
            children.0.push(root);
        }
        Ok(children)
    }

    pub(super) fn capture(leader_pid: u32) -> io::Result<Self> {
        if leader_pid == 0 || leader_pid > i32::MAX as u32 {
            return Ok(Self::default());
        }
        let leader_pid = leader_pid as i32;
        let Some(leader) = identity(leader_pid)? else {
            return Ok(Self::default());
        };
        let mut process_table = HashMap::<i32, Identity>::new();
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
            else {
                continue;
            };
            if process_table.len() >= 131_072 {
                return Err(io::Error::other(
                    "process table exceeds descendant tracking limit",
                ));
            }
            if let Ok(Some(stat)) = identity(pid) {
                process_table.insert(pid, stat);
            }
        }
        // Capture by the actual process-parent relationship, not matching
        // UID, session or process group (which could include unrelated work).
        let mut seen = HashSet::from([leader.pid]);
        let mut descendants = Vec::<Identity>::new();
        let mut frontier = vec![leader.pid];
        let mut depth = 1;
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for stat in process_table.values() {
                if frontier.contains(&stat.parent) && seen.insert(stat.pid) {
                    let mut stat = *stat;
                    stat.depth = depth;
                    descendants.push(stat);
                    next.push(stat.pid);
                }
            }
            frontier = next;
            depth += 1;
        }
        // Descendant children before their parents gives parents a chance to
        // reap them while those parents remain alive, before their own kill.
        descendants.sort_unstable_by_key(|id| std::cmp::Reverse(id.depth));
        Ok(Self(descendants))
    }

    pub(super) fn terminate_remaining(self) -> io::Result<usize> {
        let mut signaled = 0;
        for id in &self.0 {
            if !still_running(id)? {
                continue;
            }
            // Recheck identity immediately before the signal to avoid PID
            // reuse. SIGKILL targets the individual recorded PID, not -PGID.
            if still_running(id)? {
                // SAFETY: the PID was captured below the owned leader and
                // its start time has just been verified.
                let result = unsafe { libc::kill(id.pid, libc::SIGKILL) };
                if result != 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        return Err(error);
                    }
                } else {
                    signaled += 1;
                }
            }
        }
        let deadline = Instant::now() + Duration::from_millis(750);
        while Instant::now() < deadline {
            if self.0.iter().try_fold(true, |all_dead, id| {
                Ok::<_, io::Error>(all_dead && !still_running(id)?)
            })? {
                return Ok(signaled);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let live = self
            .0
            .iter()
            .filter_map(|id| {
                still_running(id)
                    .ok()
                    .and_then(|running| running.then_some(id.pid))
            })
            .collect::<Vec<_>>();
        if live.is_empty() {
            Ok(signaled)
        } else {
            Err(io::Error::other(format!(
                "owned descendant PIDs survived SIGKILL: {live:?}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_parser_handles_a_real_self_identity() {
        let current = identity(std::process::id() as i32).unwrap().unwrap();
        assert_eq!(current.pid, std::process::id() as i32);
        assert!(current.started > 0);
        assert!(still_running(&current).unwrap());
    }

    #[test]
    fn an_unrelated_process_is_never_claimed_as_a_descendant() {
        let descendants = Descendants::capture(std::process::id()).unwrap();
        assert!(
            !descendants
                .0
                .iter()
                .any(|id| id.pid == std::process::id() as i32)
        );
    }
}
