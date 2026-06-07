use std::fs;
use std::path::{Path, PathBuf};

use diff::Result as DiffResult;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::message::ApplyPatchToolInput;

use super::{ToolError, ToolExecutor};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedFileChange {
    pub path: String,
    pub kind: PatchOpKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyPatchExecution {
    pub operation_id: String,
    pub files: Vec<AppliedFileChange>,
    pub before_hash: String,
    pub after_hash: String,
    pub inverse_patch: String,
    pub diff: String,
    pub progress: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOpKind {
    Add,
    Update,
    Delete,
    Move,
}

#[derive(Debug, Clone)]
enum PatchOp {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        hunks: Vec<Hunk>,
    },
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
    let mut diff_sections = Vec::new();
    let mut progress = ops.iter().map(describe_planned_op).collect::<Vec<_>>();

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
                diff_sections.push(render_add_diff(&path, &content));
                progress.push(format!("applied add {path}"));
                changed_files.push(AppliedFileChange {
                    path,
                    kind: PatchOpKind::Add,
                    from_path: None,
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
                diff_sections.push(render_delete_diff(&path, &original));
                progress.push(format!("applied delete {path}"));
                changed_files.push(AppliedFileChange {
                    path,
                    kind: PatchOpKind::Delete,
                    from_path: None,
                });
            }
            PatchOp::Update {
                path,
                move_to,
                hunks,
            } => {
                let source = absolute_path(executor.workspace_root(), &path);
                executor.ensure_edit_permission(&source)?;
                if !source.exists() {
                    return Err(ToolError::InvalidPatch(format!(
                        "update file target does not exist: {path}"
                    )));
                }

                let original = fs::read_to_string(&source)?;
                let updated = apply_hunks(&path, &original, &hunks)?;

                if let Some(target_path) = move_to {
                    let target = absolute_path(executor.workspace_root(), &target_path);
                    executor.ensure_edit_permission(&target)?;
                    ensure_parent(&target)?;
                    if source != target && target.exists() {
                        return Err(ToolError::InvalidPatch(format!(
                            "move target already exists: {target_path}"
                        )));
                    }

                    before_state.push_str(&format!("M:{path}:{original}\n"));
                    after_state.push_str(&format!("M:{target_path}:{updated}\n"));

                    if source == target {
                        fs::write(&source, &updated)?;
                    } else {
                        fs::write(&target, &updated)?;
                        fs::remove_file(&source)?;
                    }

                    inverse_sections.push(render_update_inverse_section(
                        &path,
                        Some(&target_path),
                        &updated,
                        &original,
                    ));
                    diff_sections.push(render_update_diff(&path, Some(&target_path), &hunks));
                    progress.push(format!("applied move {path} -> {target_path}"));
                    changed_files.push(AppliedFileChange {
                        path: target_path,
                        kind: PatchOpKind::Move,
                        from_path: Some(path),
                    });
                } else {
                    let absolute = source;
                    before_state.push_str(&format!("U:{path}:{original}\n"));
                    after_state.push_str(&format!("U:{path}:{updated}\n"));

                    fs::write(&absolute, &updated)?;

                    inverse_sections.push(render_update_inverse_section(
                        &path, None, &updated, &original,
                    ));
                    diff_sections.push(render_update_diff(&path, None, &hunks));
                    progress.push(format!("applied update {path}"));
                    changed_files.push(AppliedFileChange {
                        path,
                        kind: PatchOpKind::Update,
                        from_path: None,
                    });
                }
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
        diff: diff_sections.join("\n"),
        progress,
    })
}

pub(crate) fn planned_paths(text: &str) -> Result<Vec<String>, ToolError> {
    let ops = parse_patch(text)?;
    let mut paths = Vec::new();
    for op in ops {
        match op {
            PatchOp::Add { path, .. } | PatchOp::Delete { path } => paths.push(path),
            PatchOp::Update { path, move_to, .. } => {
                paths.push(path);
                if let Some(target) = move_to {
                    paths.push(target);
                }
            }
        }
    }
    Ok(paths)
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
            let mut move_to = None;
            let mut hunks = Vec::new();
            let mut old_lines = Vec::new();
            let mut new_lines = Vec::new();

            while idx < lines.len() - 1 {
                let l = lines[idx];
                if let Some(target) = l.strip_prefix("*** Move to: ") {
                    if move_to.replace(target.to_string()).is_some() {
                        return Err(ToolError::InvalidPatch(format!(
                            "duplicate move target for update file: {path}"
                        )));
                    }
                    idx += 1;
                    continue;
                }
                if l.starts_with("*** ") {
                    break;
                }
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
                    "invalid update hunk line in {path}: {l}"
                )));
            }

            if !old_lines.is_empty() || !new_lines.is_empty() {
                hunks.push(Hunk {
                    old: join_lines(&old_lines),
                    new: join_lines(&new_lines),
                });
            }

            if hunks.is_empty() && move_to.is_none() {
                return Err(ToolError::InvalidPatch(format!(
                    "update file has no hunks: {path}"
                )));
            }

            ops.push(PatchOp::Update {
                path: path.to_string(),
                move_to,
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

fn apply_hunks(path: &str, original: &str, hunks: &[Hunk]) -> Result<String, ToolError> {
    let mut content = normalize_lf(original);
    for (index, hunk) in hunks.iter().enumerate() {
        if hunk.old.is_empty() {
            content.push_str(&hunk.new);
            continue;
        }
        let pos = content.find(&hunk.old).ok_or_else(|| {
            ToolError::InvalidPatch(format!(
                "failed to locate update hunk {} in target file: {path}",
                index + 1
            ))
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

fn render_update_inverse_section(
    original_path: &str,
    current_path: Option<&str>,
    now_content: &str,
    before_content: &str,
) -> String {
    let mut lines = vec![format!(
        "*** Update File: {}",
        current_path.unwrap_or(original_path)
    )];
    if current_path.is_some() {
        lines.push(format!("*** Move to: {original_path}"));
    }
    if now_content != before_content {
        lines.push("@@".to_string());
        for line in normalize_lf(now_content).lines() {
            lines.push(format!("-{line}"));
        }
        for line in normalize_lf(before_content).lines() {
            lines.push(format!("+{line}"));
        }
    }
    lines.join("\n")
}

fn render_add_diff(path: &str, content: &str) -> String {
    let mut lines = vec![format!("diff --git a/{} b/{}", path, path)];
    lines.push("new file mode 100644".to_string());
    lines.push("--- /dev/null".to_string());
    lines.push(format!("+++ b/{path}"));
    lines.push("@@".to_string());
    push_prefixed_lines(&mut lines, '+', content);
    lines.join("\n")
}

fn render_delete_diff(path: &str, content: &str) -> String {
    let mut lines = vec![format!("diff --git a/{path} b/{path}")];
    lines.push("deleted file mode 100644".to_string());
    lines.push(format!("--- a/{path}"));
    lines.push("+++ /dev/null".to_string());
    lines.push("@@".to_string());
    push_prefixed_lines(&mut lines, '-', content);
    lines.join("\n")
}

fn render_update_diff(path: &str, move_to: Option<&str>, hunks: &[Hunk]) -> String {
    let target_path = move_to.unwrap_or(path);
    let mut lines = vec![format!("diff --git a/{path} b/{target_path}")];
    if let Some(target_path) = move_to {
        lines.push(format!("rename from {path}"));
        lines.push(format!("rename to {target_path}"));
    }
    lines.push(format!("--- a/{path}"));
    lines.push(format!("+++ b/{target_path}"));

    if hunks.is_empty() {
        return lines.join("\n");
    }

    for hunk in hunks {
        lines.push("@@".to_string());
        for line in diff::lines(&hunk.old, &hunk.new) {
            match line {
                DiffResult::Left(value) => lines.push(format!("-{value}")),
                DiffResult::Both(value, _) => lines.push(format!(" {value}")),
                DiffResult::Right(value) => lines.push(format!("+{value}")),
            }
        }
    }

    lines.join("\n")
}

fn push_prefixed_lines(lines: &mut Vec<String>, prefix: char, content: &str) {
    let normalized = normalize_lf(content);
    for line in normalized.lines() {
        lines.push(format!("{prefix}{line}"));
    }
    if normalized.ends_with('\n') {
        lines.push(prefix.to_string());
    }
}

fn describe_planned_op(op: &PatchOp) -> String {
    match op {
        PatchOp::Add { path, .. } => format!("parsed add {path}"),
        PatchOp::Delete { path } => format!("parsed delete {path}"),
        PatchOp::Update { path, move_to, .. } => match move_to {
            Some(target) => format!("parsed move {path} -> {target}"),
            None => format!("parsed update {path}"),
        },
    }
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
