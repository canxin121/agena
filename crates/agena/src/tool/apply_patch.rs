use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::message::ApplyPatchToolInput;

use super::{ToolError, ToolExecutor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedFileChange {
    pub path: String,
    pub kind: PatchOpKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPatchExecution {
    pub operation_id: String,
    pub files: Vec<AppliedFileChange>,
    pub before_hash: String,
    pub after_hash: String,
    pub inverse_patch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchOpKind {
    Add,
    Update,
    Delete,
}

#[derive(Debug, Clone)]
enum PatchOp {
    Add { path: String, content: String },
    Delete { path: String },
    Update { path: String, hunks: Vec<Hunk> },
}

#[derive(Debug, Clone)]
struct Hunk {
    old: String,
    new: String,
}

pub fn execute(
    executor: &ToolExecutor,
    input: &ApplyPatchToolInput,
) -> Result<ApplyPatchExecution, ToolError> {
    let ops = parse_patch(&input.patch)?;
    if ops.is_empty() {
        return Err(ToolError::InvalidPatch(
            "no operations in patch".to_string(),
        ));
    }

    let mut before_state = String::new();
    let mut after_state = String::new();
    let mut inverse_sections = Vec::new();
    let mut changed_files = Vec::new();

    for op in ops {
        match op {
            PatchOp::Add { path, content } => {
                let absolute = absolute_path(executor.workspace_root(), &path);
                executor.ensure_edit_permission(&absolute)?;
                ensure_parent(&absolute)?;
                if absolute.exists() {
                    return Err(ToolError::InvalidPatch(format!(
                        "add file target already exists: {path}"
                    )));
                }

                before_state.push_str(&format!("A:{path}:<missing>\n"));
                after_state.push_str(&format!("A:{path}:{content}\n"));

                fs::write(&absolute, &content)?;

                inverse_sections.push(format!("*** Delete File: {path}"));
                changed_files.push(AppliedFileChange {
                    path,
                    kind: PatchOpKind::Add,
                });
            }
            PatchOp::Delete { path } => {
                let absolute = absolute_path(executor.workspace_root(), &path);
                executor.ensure_edit_permission(&absolute)?;
                if !absolute.exists() {
                    return Err(ToolError::InvalidPatch(format!(
                        "delete file target does not exist: {path}"
                    )));
                }

                let original = fs::read_to_string(&absolute)?;
                before_state.push_str(&format!("D:{path}:{original}\n"));
                after_state.push_str(&format!("D:{path}:<deleted>\n"));

                fs::remove_file(&absolute)?;

                inverse_sections.push(render_add_file_section(&path, &original));
                changed_files.push(AppliedFileChange {
                    path,
                    kind: PatchOpKind::Delete,
                });
            }
            PatchOp::Update { path, hunks } => {
                let absolute = absolute_path(executor.workspace_root(), &path);
                executor.ensure_edit_permission(&absolute)?;
                if !absolute.exists() {
                    return Err(ToolError::InvalidPatch(format!(
                        "update file target does not exist: {path}"
                    )));
                }

                let original = fs::read_to_string(&absolute)?;
                let updated = apply_hunks(&original, &hunks)?;

                before_state.push_str(&format!("U:{path}:{original}\n"));
                after_state.push_str(&format!("U:{path}:{updated}\n"));

                fs::write(&absolute, &updated)?;

                inverse_sections.push(render_update_inverse_section(&path, &updated, &original));
                changed_files.push(AppliedFileChange {
                    path,
                    kind: PatchOpKind::Update,
                });
            }
        }
    }

    let operation_id = Uuid::new_v4().to_string();
    let inverse_patch = format!(
        "*** Begin Patch\n{}\n*** End Patch",
        inverse_sections.join("\n")
    );

    Ok(ApplyPatchExecution {
        operation_id,
        files: changed_files,
        before_hash: sha256_hex(&before_state),
        after_hash: sha256_hex(&after_state),
        inverse_patch,
    })
}

pub(crate) fn planned_paths(text: &str) -> Result<Vec<String>, ToolError> {
    let ops = parse_patch(text)?;
    Ok(ops
        .into_iter()
        .map(|op| match op {
            PatchOp::Add { path, .. } | PatchOp::Delete { path } | PatchOp::Update { path, .. } => {
                path
            }
        })
        .collect())
}

fn parse_patch(text: &str) -> Result<Vec<PatchOp>, ToolError> {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return Err(ToolError::InvalidPatch("empty patch".to_string()));
    }

    if lines.first().copied() != Some("*** Begin Patch") {
        return Err(ToolError::InvalidPatch(
            "patch must start with '*** Begin Patch'".to_string(),
        ));
    }
    if lines.last().copied() != Some("*** End Patch") {
        return Err(ToolError::InvalidPatch(
            "patch must end with '*** End Patch'".to_string(),
        ));
    }

    let mut idx = 1usize;
    let mut ops = Vec::new();

    while idx < lines.len() - 1 {
        let line = lines[idx];
        if line.trim().is_empty() {
            idx += 1;
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Add File: ") {
            idx += 1;
            let mut content = Vec::new();
            while idx < lines.len() - 1 && !lines[idx].starts_with("*** ") {
                let l = lines[idx];
                let payload = l.strip_prefix('+').ok_or_else(|| {
                    ToolError::InvalidPatch("add file expects '+' prefixed lines".to_string())
                })?;
                content.push(payload);
                idx += 1;
            }
            ops.push(PatchOp::Add {
                path: path.to_string(),
                content: join_lines(&content),
            });
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            idx += 1;
            ops.push(PatchOp::Delete {
                path: path.to_string(),
            });
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Update File: ") {
            idx += 1;
            let mut hunks = Vec::new();
            let mut old_lines = Vec::new();
            let mut new_lines = Vec::new();

            while idx < lines.len() - 1 && !lines[idx].starts_with("*** ") {
                let l = lines[idx];
                if l.starts_with("@@") {
                    if !old_lines.is_empty() || !new_lines.is_empty() {
                        hunks.push(Hunk {
                            old: join_lines(&old_lines),
                            new: join_lines(&new_lines),
                        });
                        old_lines.clear();
                        new_lines.clear();
                    }
                    idx += 1;
                    continue;
                }

                if let Some(payload) = l.strip_prefix(' ') {
                    old_lines.push(payload);
                    new_lines.push(payload);
                    idx += 1;
                    continue;
                }
                if let Some(payload) = l.strip_prefix('-') {
                    old_lines.push(payload);
                    idx += 1;
                    continue;
                }
                if let Some(payload) = l.strip_prefix('+') {
                    new_lines.push(payload);
                    idx += 1;
                    continue;
                }

                return Err(ToolError::InvalidPatch(format!(
                    "invalid update hunk line: {l}"
                )));
            }

            if !old_lines.is_empty() || !new_lines.is_empty() {
                hunks.push(Hunk {
                    old: join_lines(&old_lines),
                    new: join_lines(&new_lines),
                });
            }

            if hunks.is_empty() {
                return Err(ToolError::InvalidPatch(format!(
                    "update file has no hunks: {path}"
                )));
            }

            ops.push(PatchOp::Update {
                path: path.to_string(),
                hunks,
            });
            continue;
        }

        return Err(ToolError::InvalidPatch(format!(
            "unknown patch section header: {line}"
        )));
    }

    Ok(ops)
}

fn apply_hunks(original: &str, hunks: &[Hunk]) -> Result<String, ToolError> {
    let mut content = normalize_lf(original);
    for hunk in hunks {
        if hunk.old.is_empty() {
            content.push_str(&hunk.new);
            continue;
        }
        let pos = content.find(&hunk.old).ok_or_else(|| {
            ToolError::InvalidPatch("failed to locate update hunk in target file".to_string())
        })?;
        content.replace_range(pos..(pos + hunk.old.len()), &hunk.new);
    }

    if original.contains("\r\n") {
        Ok(content.replace('\n', "\r\n"))
    } else {
        Ok(content)
    }
}

fn render_add_file_section(path: &str, content: &str) -> String {
    let mut lines = vec![format!("*** Add File: {path}")];
    for line in normalize_lf(content).lines() {
        lines.push(format!("+{line}"));
    }
    if content.ends_with('\n') {
        lines.push("+".to_string());
    }
    lines.join("\n")
}

fn render_update_inverse_section(path: &str, now_content: &str, before_content: &str) -> String {
    let mut lines = vec![format!("*** Update File: {path}"), "@@".to_string()];
    for line in normalize_lf(now_content).lines() {
        lines.push(format!("-{line}"));
    }
    for line in normalize_lf(before_content).lines() {
        lines.push(format!("+{line}"));
    }
    lines.join("\n")
}

fn absolute_path(workspace_root: &Path, file_path: &str) -> PathBuf {
    let path = PathBuf::from(file_path);
    if path.is_absolute() {
        path
    } else {
        workspace_root.join(path)
    }
}

fn ensure_parent(path: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn join_lines(lines: &[&str]) -> String {
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn normalize_lf(input: &str) -> String {
    input.replace("\r\n", "\n")
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(nibble_to_hex((byte >> 4) & 0x0f));
        out.push(nibble_to_hex(byte & 0x0f));
    }
    out
}

fn nibble_to_hex(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        10..=15 => (b'a' + (v - 10)) as char,
        _ => '0',
    }
}
