use std::collections::BTreeMap;
use std::fs::{self, Permissions};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use similar::TextDiff;

mod edit;
mod format;
use edit::apply_hunks;
use format::parse_patch;
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

#[derive(Debug, Clone, Default)]
struct Hunk {
    old: String,
    new: String,
    contexts: Vec<String>,
    end_of_file: bool,
    old_no_newline: bool,
    new_no_newline: bool,
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
    },
    Move {
        path: String,
        target_path: String,
        source: PathBuf,
        target: PathBuf,
        original: String,
        updated: String,
        permissions: Permissions,
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
    // mutation so a deterministic validation failure cannot partially apply.
    let prepared = prepare_operations(ops, |path| executor.resolve_target_path(path))?;

    let mut changed_files = Vec::new();
    let mut diff_sections = Vec::new();
    let mut progress = prepared
        .iter()
        .map(describe_prepared_op)
        .collect::<Vec<_>>();

    for op in &prepared {
        match op {
            PreparedPatchOp::Add { path, content, .. } => {
                diff_sections.push(render_add_diff(path, content));
                changed_files.push(AppliedFileChange {
                    path: path.clone(),
                    kind: PatchOpKind::Add,
                    from_path: None,
                });
            }
            PreparedPatchOp::Delete { path, original, .. } => {
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
                ..
            } => {
                diff_sections.push(render_update_diff(path, None, original, updated));
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
                ..
            } => {
                diff_sections.push(render_update_diff(
                    path,
                    Some(target_path),
                    original,
                    updated,
                ));
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
    Ok(ApplyPatchExecution {
        operation_id,
        files: changed_files,
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
                    });
                } else {
                    prepared.push(PreparedPatchOp::Update {
                        path,
                        absolute: source,
                        original,
                        updated,
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
    // Best-effort cleanup of a failed tool invocation, not a durable undo
    // history. Recovery after a successful edit belongs to the Git workflow.
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
        PreparedPatchOp::Delete {
            absolute, original, ..
        } => remove_revision(absolute, original),
        PreparedPatchOp::Update {
            absolute,
            updated,
            original,
            ..
        } => replace_revision(absolute, updated, original),
        PreparedPatchOp::Move {
            source,
            target,
            updated,
            original,
            ..
        } if source == target => replace_revision(source, updated, original),
        PreparedPatchOp::Move {
            source,
            target,
            updated,
            original,
            permissions,
            ..
        } => {
            ensure_parent(target)?;
            crate::verify_file_contents(source, original.as_bytes())?;
            crate::atomic_create_file(target, updated.as_bytes(), Some(permissions.clone()))?;
            if let Err(error) = remove_revision(source, original) {
                return match remove_revision(target, updated) {
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

fn replace_revision(path: &Path, updated: &str, expected: &str) -> std::io::Result<()> {
    crate::atomic_replace_file_with_check(path, updated.as_bytes(), || {
        crate::verify_file_contents(path, expected.as_bytes())
    })
}

fn remove_revision(path: &Path, expected: &str) -> std::io::Result<()> {
    crate::verify_file_contents(path, expected.as_bytes())?;
    fs::remove_file(path)
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
        PreparedPatchOp::Add {
            absolute, content, ..
        } => remove_revision(absolute, content),
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
            absolute,
            original,
            updated,
            ..
        } => replace_revision(absolute, original, updated),
        PreparedPatchOp::Move {
            source,
            target,
            original,
            updated,
            ..
        } if source == target => replace_revision(source, original, updated),
        PreparedPatchOp::Move {
            source,
            target,
            original,
            updated,
            permissions,
            ..
        } => {
            ensure_parent(source)?;
            crate::verify_file_contents(target, updated.as_bytes())?;
            crate::atomic_create_file(source, original.as_bytes(), Some(permissions.clone()))?;
            remove_revision(target, updated)
        }
    }
}

/// Every path a patch will touch, in patch order. Used by the host's own
/// permission preflight for `apply_patch`, which resolves the paths of the
/// patch text it is about to apply.
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

fn render_add_diff(path: &str, content: &str) -> String {
    format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n{}",
        TextDiff::from_lines("", content)
            .unified_diff()
            .header("/dev/null", &format!("b/{path}"))
    )
}

fn render_delete_diff(path: &str, content: &str) -> String {
    format!(
        "diff --git a/{path} b/{path}\ndeleted file mode 100644\n{}",
        TextDiff::from_lines(content, "")
            .unified_diff()
            .header(&format!("a/{path}"), "/dev/null")
    )
}

fn render_update_diff(path: &str, move_to: Option<&str>, original: &str, updated: &str) -> String {
    let target = move_to.unwrap_or(path);
    let mut output = format!("diff --git a/{path} b/{target}\n");
    if move_to.is_some() {
        output.push_str(&format!("rename from {path}\nrename to {target}\n"));
    }
    output.push_str(
        &TextDiff::from_lines(original, updated)
            .unified_diff()
            .header(&format!("a/{path}"), &format!("b/{target}"))
            .to_string(),
    );
    output
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
    let file = agena_tool::file_io::open_regular_file(path)?;
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
