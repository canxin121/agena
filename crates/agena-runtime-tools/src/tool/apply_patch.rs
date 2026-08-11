use std::collections::BTreeMap;
use std::fs::{self, Permissions};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use uuid::Uuid;

use crate::part::ApplyPatchToolInput;

use super::{ToolError, ToolExecutor};
use agena_tool::{AppliedFileChange, ApplyPatchExecution, PatchOpKind};

const MAX_PATCH_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PATCH_OPERATIONS: usize = 256;
const MAX_PATCH_TRANSACTION_BYTES: usize = 64 * 1024 * 1024;

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

#[derive(Debug)]
enum PreparedPatchOp {
    Add {
        path: String,
        absolute: PathBuf,
        content: String,
    },
    Delete {
        path: String,
        absolute: PathBuf,
        original: String,
        permissions: Permissions,
    },
    Update {
        path: String,
        absolute: PathBuf,
        original: String,
        updated: String,
        hunks: Vec<Hunk>,
    },
    Move {
        path: String,
        target_path: String,
        source: PathBuf,
        target: PathBuf,
        original: String,
        updated: String,
        permissions: Permissions,
        hunks: Vec<Hunk>,
    },
}

pub fn execute(
    executor: &ToolExecutor,
    input: &ApplyPatchToolInput,
) -> Result<ApplyPatchExecution, ToolError> {
    let ops = parse_patch(&input.patch)?;
    if ops.is_empty() {
        return Err(ToolError::invalid_patch(
            "no operations in patch".to_string(),
        ));
    }
    if ops.len() > MAX_PATCH_OPERATIONS {
        return Err(ToolError::invalid_patch(format!(
            "patch contains {} operations; the limit is {MAX_PATCH_OPERATIONS}",
            ops.len()
        )));
    }

    let lock_paths = ops
        .iter()
        .flat_map(|op| match op {
            PatchOp::Add { path, .. } | PatchOp::Delete { path } => {
                vec![executor.resolve_target_path(path)]
            }
            PatchOp::Update { path, move_to, .. } => {
                let mut paths = vec![executor.resolve_target_path(path)];
                if let Some(target) = move_to {
                    paths.push(executor.resolve_target_path(target));
                }
                paths
            }
        })
        .collect::<Vec<_>>();

    crate::with_file_mutation_locks(lock_paths.as_slice(), || execute_locked(executor, ops))?
}

fn execute_locked(
    executor: &ToolExecutor,
    ops: Vec<PatchOp>,
) -> Result<ApplyPatchExecution, ToolError> {
    // Resolve every hunk and validate every source/target before the first
    // mutation. This removes the old deterministic partial-apply failure mode.
    let prepared = prepare_operations(ops, |path| executor.resolve_target_path(path))?;

    let mut before_state = String::new();
    let mut after_state = String::new();
    let mut inverse_sections = Vec::new();
    let mut changed_files = Vec::new();
    let mut diff_sections = Vec::new();
    let mut progress = prepared
        .iter()
        .map(describe_prepared_op)
        .collect::<Vec<_>>();

    for op in &prepared {
        match op {
            PreparedPatchOp::Add { path, content, .. } => {
                before_state.push_str(&format!("A:{path}:<missing>\n"));
                after_state.push_str(&format!("A:{path}:{content}\n"));
                inverse_sections.push(format!("*** Delete File: {path}"));
                diff_sections.push(render_add_diff(path, content));
                changed_files.push(AppliedFileChange {
                    path: path.clone(),
                    kind: PatchOpKind::Add,
                    from_path: None,
                });
            }
            PreparedPatchOp::Delete { path, original, .. } => {
                before_state.push_str(&format!("D:{path}:{original}\n"));
                after_state.push_str(&format!("D:{path}:<deleted>\n"));
                inverse_sections.push(render_add_file_section(path, original));
                diff_sections.push(render_delete_diff(path, original));
                changed_files.push(AppliedFileChange {
                    path: path.clone(),
                    kind: PatchOpKind::Delete,
                    from_path: None,
                });
            }
            PreparedPatchOp::Update {
                path,
                original,
                updated,
                hunks,
                ..
            } => {
                before_state.push_str(&format!("U:{path}:{original}\n"));
                after_state.push_str(&format!("U:{path}:{updated}\n"));
                inverse_sections.push(render_update_inverse_section(path, None, updated, original));
                diff_sections.push(render_update_diff(path, None, hunks));
                changed_files.push(AppliedFileChange {
                    path: path.clone(),
                    kind: PatchOpKind::Update,
                    from_path: None,
                });
            }
            PreparedPatchOp::Move {
                path,
                target_path,
                original,
                updated,
                hunks,
                ..
            } => {
                before_state.push_str(&format!("M:{path}:{original}\n"));
                after_state.push_str(&format!("M:{target_path}:{updated}\n"));
                inverse_sections.push(render_update_inverse_section(
                    path,
                    Some(target_path),
                    updated,
                    original,
                ));
                diff_sections.push(render_update_diff(path, Some(target_path), hunks));
                changed_files.push(AppliedFileChange {
                    path: target_path.clone(),
                    kind: PatchOpKind::Move,
                    from_path: Some(path.clone()),
                });
            }
        }
    }

    commit_operations(&prepared)?;
    progress.extend(prepared.iter().map(describe_applied_op));

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

fn prepare_operations(
    ops: Vec<PatchOp>,
    resolve: impl Fn(&str) -> PathBuf,
) -> Result<Vec<PreparedPatchOp>, ToolError> {
    let mut prepared = Vec::with_capacity(ops.len());
    let mut claimed_paths = BTreeMap::<PathBuf, String>::new();
    let mut transaction_bytes = 0_usize;

    for op in ops {
        match op {
            PatchOp::Add { path, content } => {
                let absolute = resolve(path.as_str());
                claim_path(&mut claimed_paths, &absolute, path.as_str())?;
                if absolute.exists() {
                    return Err(ToolError::invalid_patch(format!(
                        "add file target already exists: {path}"
                    )));
                }
                validate_parent_chain(&absolute)?;
                add_transaction_bytes(&mut transaction_bytes, content.len())?;
                prepared.push(PreparedPatchOp::Add {
                    path,
                    absolute,
                    content,
                });
            }
            PatchOp::Delete { path } => {
                let absolute = resolve(path.as_str());
                claim_path(&mut claimed_paths, &absolute, path.as_str())?;
                let (original, permissions) = read_existing_patch_file(&absolute, path.as_str())?;
                add_transaction_bytes(&mut transaction_bytes, original.len())?;
                prepared.push(PreparedPatchOp::Delete {
                    path,
                    absolute,
                    original,
                    permissions,
                });
            }
            PatchOp::Update {
                path,
                move_to,
                hunks,
            } => {
                let source = resolve(path.as_str());
                claim_path(&mut claimed_paths, &source, path.as_str())?;
                let (original, permissions) = read_existing_patch_file(&source, path.as_str())?;
                let updated = apply_hunks(path.as_str(), &original, &hunks)?;
                add_transaction_bytes(
                    &mut transaction_bytes,
                    original.len().saturating_add(updated.len()),
                )?;

                if let Some(target_path) = move_to {
                    let target = resolve(target_path.as_str());
                    if source != target {
                        claim_path(&mut claimed_paths, &target, target_path.as_str())?;
                        if target.exists() {
                            return Err(ToolError::invalid_patch(format!(
                                "move target already exists: {target_path}"
                            )));
                        }
                        validate_parent_chain(&target)?;
                    }
                    prepared.push(PreparedPatchOp::Move {
                        path,
                        target_path,
                        source,
                        target,
                        original,
                        updated,
                        permissions,
                        hunks,
                    });
                } else {
                    prepared.push(PreparedPatchOp::Update {
                        path,
                        absolute: source,
                        original,
                        updated,
                        hunks,
                    });
                }
            }
        }
    }
    Ok(prepared)
}

fn claim_path(
    claimed: &mut BTreeMap<PathBuf, String>,
    absolute: &Path,
    display: &str,
) -> Result<(), ToolError> {
    if let Some(previous) = claimed.insert(absolute.to_path_buf(), display.to_string()) {
        return Err(ToolError::invalid_patch(format!(
            "patch targets the same file more than once: '{previous}' and '{display}'"
        )));
    }
    Ok(())
}

fn read_existing_patch_file(
    absolute: &Path,
    display: &str,
) -> Result<(String, Permissions), ToolError> {
    let metadata = fs::metadata(absolute).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ToolError::invalid_patch(format!("patch target does not exist: {display}"))
        } else {
            ToolError::Io(error)
        }
    })?;
    if !metadata.is_file() {
        return Err(ToolError::invalid_patch(format!(
            "patch target is not a regular file: {display}"
        )));
    }
    Ok((read_patch_target(absolute)?, metadata.permissions()))
}

fn validate_parent_chain(path: &Path) -> Result<(), ToolError> {
    let mut current = path.parent().ok_or_else(|| {
        ToolError::invalid_patch(format!("patch target has no parent: {}", path.display()))
    })?;
    while !current.exists() {
        current = current.parent().ok_or_else(|| {
            ToolError::invalid_patch(format!(
                "patch target has no existing parent: {}",
                path.display()
            ))
        })?;
    }
    if !current.is_dir() {
        return Err(ToolError::invalid_patch(format!(
            "patch target parent is not a directory: {}",
            current.display()
        )));
    }
    Ok(())
}

fn add_transaction_bytes(total: &mut usize, bytes: usize) -> Result<(), ToolError> {
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| ToolError::invalid_patch("patch transaction byte accounting overflowed"))?;
    if *total > MAX_PATCH_TRANSACTION_BYTES {
        return Err(ToolError::invalid_patch(format!(
            "patch transaction exceeds the {} MiB aggregate text limit",
            MAX_PATCH_TRANSACTION_BYTES / 1024 / 1024
        )));
    }
    Ok(())
}

fn commit_operations(ops: &[PreparedPatchOp]) -> Result<(), ToolError> {
    let mut committed = Vec::with_capacity(ops.len());
    for (index, op) in ops.iter().enumerate() {
        if let Err(commit_error) = commit_operation(op) {
            let rollback_errors = rollback_operations(ops, committed.as_slice());
            if rollback_errors.is_empty() {
                return Err(ToolError::Io(commit_error));
            }
            return Err(ToolError::Io(std::io::Error::other(format!(
                "patch commit failed: {commit_error}; rollback was incomplete: {}",
                rollback_errors.join("; ")
            ))));
        }
        committed.push(index);
    }
    Ok(())
}

fn commit_operation(op: &PreparedPatchOp) -> std::io::Result<()> {
    match op {
        PreparedPatchOp::Add {
            absolute, content, ..
        } => {
            ensure_parent(absolute)?;
            crate::atomic_create_file(absolute, content.as_bytes(), None)
        }
        PreparedPatchOp::Delete { absolute, .. } => fs::remove_file(absolute),
        PreparedPatchOp::Update {
            absolute, updated, ..
        } => crate::atomic_replace_file(absolute, updated.as_bytes()),
        PreparedPatchOp::Move {
            source,
            target,
            updated,
            ..
        } if source == target => crate::atomic_replace_file(source, updated.as_bytes()),
        PreparedPatchOp::Move {
            source,
            target,
            updated,
            permissions,
            ..
        } => {
            ensure_parent(target)?;
            crate::atomic_create_file(target, updated.as_bytes(), Some(permissions.clone()))?;
            if let Err(error) = fs::remove_file(source) {
                return match fs::remove_file(target) {
                    Ok(()) => Err(error),
                    Err(cleanup_error) => Err(std::io::Error::other(format!(
                        "could not remove move source ({error}) and could not clean up staged target ({cleanup_error})"
                    ))),
                };
            }
            Ok(())
        }
    }
}

fn rollback_operations(ops: &[PreparedPatchOp], committed: &[usize]) -> Vec<String> {
    let mut errors = Vec::new();
    for index in committed.iter().rev().copied() {
        if let Err(error) = rollback_operation(&ops[index]) {
            errors.push(format!("{}: {error}", describe_prepared_op(&ops[index])));
        }
    }
    errors
}

fn rollback_operation(op: &PreparedPatchOp) -> std::io::Result<()> {
    match op {
        PreparedPatchOp::Add { absolute, .. } => fs::remove_file(absolute),
        PreparedPatchOp::Delete {
            absolute,
            original,
            permissions,
            ..
        } => {
            ensure_parent(absolute)?;
            crate::atomic_create_file(absolute, original.as_bytes(), Some(permissions.clone()))
        }
        PreparedPatchOp::Update {
            absolute, original, ..
        } => crate::atomic_replace_file(absolute, original.as_bytes()),
        PreparedPatchOp::Move {
            source,
            target,
            original,
            ..
        } if source == target => crate::atomic_replace_file(source, original.as_bytes()),
        PreparedPatchOp::Move {
            source,
            target,
            original,
            permissions,
            ..
        } => {
            ensure_parent(source)?;
            crate::atomic_create_file(source, original.as_bytes(), Some(permissions.clone()))?;
            fs::remove_file(target)
        }
    }
}

pub(crate) fn planned_paths(text: &str) -> Result<Vec<String>, ToolError> {
    let ops = parse_patch(text)?;
    if ops.len() > MAX_PATCH_OPERATIONS {
        return Err(ToolError::invalid_patch(format!(
            "patch contains {} operations; the limit is {MAX_PATCH_OPERATIONS}",
            ops.len()
        )));
    }
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
        return Err(ToolError::invalid_patch("empty patch".to_string()));
    }

    if lines.first().copied() != Some("*** Begin Patch") {
        return Err(ToolError::invalid_patch(
            "patch must start with '*** Begin Patch'".to_string(),
        ));
    }
    if lines.last().copied() != Some("*** End Patch") {
        return Err(ToolError::invalid_patch(
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
                    ToolError::invalid_patch("add file expects '+' prefixed lines".to_string())
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
                        return Err(ToolError::invalid_patch(format!(
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

                return Err(ToolError::invalid_patch(format!(
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
                return Err(ToolError::invalid_patch(format!(
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

        return Err(ToolError::invalid_patch(format!(
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
            ToolError::invalid_patch(format!(
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
        for change in TextDiff::from_lines(&hunk.old, &hunk.new).iter_all_changes() {
            let value = change.value().trim_end_matches(['\r', '\n']);
            match change.tag() {
                ChangeTag::Delete => lines.push(format!("-{value}")),
                ChangeTag::Equal => lines.push(format!(" {value}")),
                ChangeTag::Insert => lines.push(format!("+{value}")),
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

fn describe_prepared_op(op: &PreparedPatchOp) -> String {
    match op {
        PreparedPatchOp::Add { path, .. } => format!("prepared add {path}"),
        PreparedPatchOp::Delete { path, .. } => format!("prepared delete {path}"),
        PreparedPatchOp::Update { path, .. } => format!("prepared update {path}"),
        PreparedPatchOp::Move {
            path, target_path, ..
        } => format!("prepared move {path} -> {target_path}"),
    }
}

fn describe_applied_op(op: &PreparedPatchOp) -> String {
    match op {
        PreparedPatchOp::Add { path, .. } => format!("applied add {path}"),
        PreparedPatchOp::Delete { path, .. } => format!("applied delete {path}"),
        PreparedPatchOp::Update { path, .. } => format!("applied update {path}"),
        PreparedPatchOp::Move {
            path, target_path, ..
        } => format!("applied move {path} -> {target_path}"),
    }
}

fn ensure_parent(path: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn read_patch_target(path: &Path) -> Result<String, ToolError> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(
        file.metadata()
            .ok()
            .and_then(|metadata| usize::try_from(metadata.len().min(MAX_PATCH_FILE_BYTES)).ok())
            .unwrap_or_default(),
    );
    file.take(MAX_PATCH_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PATCH_FILE_BYTES {
        return Err(ToolError::invalid_patch(format!(
            "patch target exceeds the {} MiB limit: {}",
            MAX_PATCH_FILE_BYTES / 1024 / 1024,
            path.display()
        )));
    }
    String::from_utf8(bytes).map_err(|_| {
        ToolError::invalid_patch(format!(
            "patch target is not UTF-8 text: {}",
            path.display()
        ))
    })
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

#[cfg(test)]
mod tests {
    use super::{commit_operations, parse_patch, prepare_operations};

    fn resolve_from(root: &std::path::Path, path: &str) -> std::path::PathBuf {
        root.join(path)
    }

    #[test]
    fn every_hunk_is_preflighted_before_any_file_changes() {
        let directory = tempfile::tempdir().expect("temporary patch directory");
        let first = directory.path().join("first.txt");
        let second = directory.path().join("second.txt");
        std::fs::write(&first, "old first\n").expect("first fixture");
        std::fs::write(&second, "old second\n").expect("second fixture");
        let patch = "*** Begin Patch\n*** Update File: first.txt\n@@\n-old first\n+new first\n*** Update File: second.txt\n@@\n-not present\n+new second\n*** End Patch";

        let error = prepare_operations(parse_patch(patch).expect("parse patch"), |path| {
            resolve_from(directory.path(), path)
        })
        .expect_err("second hunk must fail preflight");

        assert!(error.to_string().contains("failed to locate update hunk 1"));
        assert_eq!(
            std::fs::read_to_string(&first).expect("unchanged first file"),
            "old first\n"
        );
    }

    #[test]
    fn commit_failure_rolls_back_already_replaced_files() {
        let directory = tempfile::tempdir().expect("temporary patch directory");
        let first = directory.path().join("first.txt");
        std::fs::write(&first, "old first\n").expect("first fixture");
        let patch = "*** Begin Patch\n*** Update File: first.txt\n@@\n-old first\n+new first\n*** Add File: blocked/new.txt\n+new file\n*** End Patch";
        let prepared = prepare_operations(parse_patch(patch).expect("parse patch"), |path| {
            resolve_from(directory.path(), path)
        })
        .expect("preflight succeeds before filesystem race");
        // Simulate an external filesystem change after preflight: the missing
        // parent path becomes a regular file, so the second commit must fail.
        std::fs::write(directory.path().join("blocked"), "not a directory")
            .expect("blocking parent fixture");

        let error = commit_operations(&prepared).expect_err("commit must fail and roll back");

        assert!(error.to_string().contains("io error"));
        assert_eq!(
            std::fs::read_to_string(&first).expect("rolled-back first file"),
            "old first\n"
        );
        assert!(!directory.path().join("blocked/new.txt").exists());
    }

    #[test]
    fn aliases_of_the_same_target_are_rejected() {
        let directory = tempfile::tempdir().expect("temporary patch directory");
        std::fs::write(directory.path().join("same.txt"), "old\n").expect("fixture");
        let patch = "*** Begin Patch\n*** Update File: same.txt\n@@\n-old\n+middle\n*** Update File: nested/../same.txt\n@@\n-old\n+new\n*** End Patch";

        let error = prepare_operations(parse_patch(patch).expect("parse patch"), |path| {
            crate::canonicalize_mutation_path(&resolve_from(directory.path(), path))
        })
        .expect_err("duplicate canonical target");

        assert!(
            error.to_string().contains("same file more than once"),
            "{error}"
        );
    }
}
