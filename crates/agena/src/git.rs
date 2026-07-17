use std::{path::Path, process::Command};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusSummary {
    pub staged: u64,
    pub unstaged: u64,
    pub untracked: u64,
    pub changed: u64,
}

pub fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

pub fn succeeds<const N: usize>(workspace_root: &Path, args: [&str; N]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .is_ok_and(|output| output.status.success())
}

pub fn parse_ahead_behind(value: Option<&str>) -> (Option<u64>, Option<u64>) {
    let Some(value) = value else {
        return (None, None);
    };
    let mut parts = value.split_whitespace();
    let behind = parts.next().and_then(|part| part.parse::<u64>().ok());
    let ahead = parts.next().and_then(|part| part.parse::<u64>().ok());
    (ahead, behind)
}

pub fn summarize_status(status: &str) -> StatusSummary {
    let mut summary = StatusSummary::default();

    for line in status.lines().filter(|line| !line.is_empty()) {
        summary.changed += 1;
        let bytes = line.as_bytes();
        let x = bytes.first().copied().unwrap_or(b' ');
        let y = bytes.get(1).copied().unwrap_or(b' ');
        if x == b'?' && y == b'?' {
            summary.untracked += 1;
            continue;
        }
        if x != b' ' {
            summary.staged += 1;
        }
        if y != b' ' {
            summary.unstaged += 1;
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::{StatusSummary, parse_ahead_behind, summarize_status};

    #[test]
    fn parses_ahead_and_behind_counts() {
        assert_eq!(parse_ahead_behind(Some("3 5")), (Some(5), Some(3)));
        assert_eq!(parse_ahead_behind(Some("invalid 5")), (Some(5), None));
        assert_eq!(parse_ahead_behind(None), (None, None));
    }

    #[test]
    fn summarizes_porcelain_status() {
        assert_eq!(
            summarize_status("M  staged.rs\n M unstaged.rs\nMM both.rs\n?? new.rs\n"),
            StatusSummary {
                staged: 2,
                unstaged: 2,
                untracked: 1,
                changed: 4,
            }
        );
    }
}
