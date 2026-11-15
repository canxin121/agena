use super::*;

fn change(path: PathBuf, original: Option<&[u8]>, updated: Option<&[u8]>) -> SandboxFileChange {
    let permissions = original.and_then(|_| {
        fs::metadata(&path)
            .ok()
            .map(|metadata| metadata.permissions())
    });
    SandboxFileChange {
        path,
        original: original.map(ToOwned::to_owned),
        updated: updated.map(ToOwned::to_owned),
        permissions,
    }
}

fn commit(changes: Vec<SandboxFileChange>) -> Result<(), ToolError> {
    let paths = changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    crate::with_file_mutation_locks(&paths, || commit_sandbox_changes(changes))?
}

#[test]
fn staged_add_update_and_delete_publish_original_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let added = root.join("added.txt");
    let updated = root.join("updated.txt");
    let deleted = root.join("deleted.txt");
    fs::write(&updated, b"before\n").unwrap();
    fs::write(&deleted, b"remove\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&updated, Permissions::from_mode(0o750)).unwrap();
    }
    commit(vec![
        change(added.clone(), None, Some(b"new\n")),
        change(updated.clone(), Some(b"before\n"), Some(b"after\n")),
        change(deleted.clone(), Some(b"remove\n"), None),
    ])
    .unwrap();
    assert_eq!(fs::read(&added).unwrap(), b"new\n");
    assert_eq!(fs::read(&updated).unwrap(), b"after\n");
    assert!(!deleted.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&updated).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }
}

#[test]
fn staged_results_validate_every_revision_before_first_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let first = root.join("first");
    let stale = root.join("stale");
    fs::write(&first, b"before").unwrap();
    fs::write(&stale, b"external edit").unwrap();
    let error = commit(vec![
        change(first.clone(), Some(b"before"), Some(b"after")),
        change(stale.clone(), Some(b"old"), Some(b"new")),
    ])
    .unwrap_err();
    assert!(matches!(
        error,
        ToolError::Io(ref error) if error.kind() == std::io::ErrorKind::InvalidData
    ));
    assert_eq!(fs::read(first).unwrap(), b"before");
    assert_eq!(fs::read(stale).unwrap(), b"external edit");
}

#[test]
fn broker_preserves_binary_payloads_used_by_file_writes() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let first = root.join("first");
    let invalid = root.join("invalid");
    commit(vec![
        change(first.clone(), None, Some(b"valid")),
        change(invalid.clone(), None, Some(&[0xff, 0xfe])),
    ])
    .unwrap();
    assert_eq!(fs::read(first).unwrap(), b"valid");
    assert_eq!(fs::read(invalid).unwrap(), [0xff, 0xfe]);
}

#[test]
fn commit_failure_rolls_back_already_published_sandbox_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let first = root.join("first");
    let readonly = root.join("readonly");
    fs::write(&readonly, b"before").unwrap();
    let original_permissions = fs::metadata(&readonly).unwrap().permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&readonly, permissions).unwrap();
    let result = commit(vec![
        change(first.clone(), None, Some(b"new")),
        change(readonly.clone(), Some(b"before"), Some(b"after")),
    ]);
    assert!(result.is_err());
    assert!(!first.exists());
    assert_eq!(fs::read(&readonly).unwrap(), b"before");
    fs::set_permissions(readonly, original_permissions).unwrap();
}

#[cfg(unix)]
#[test]
fn changed_target_identity_is_refused_before_publishing() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let original = root.join("original");
    let outside = root.join("outside");
    fs::write(&outside, b"before").unwrap();
    std::os::unix::fs::symlink(&outside, &original).unwrap();
    let error = commit(vec![change(original, Some(b"before"), Some(b"after"))]).unwrap_err();
    assert!(error.to_string().contains("regular file"));
    assert_eq!(fs::read(outside).unwrap(), b"before");
}

#[cfg(unix)]
#[test]
fn staged_move_preserves_host_captured_source_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source");
    let target = root.join("target");
    fs::write(&source, b"before").unwrap();
    fs::set_permissions(&source, Permissions::from_mode(0o750)).unwrap();
    let mut addition = change(target.clone(), None, Some(b"after"));
    addition.permissions = Some(fs::metadata(&source).unwrap().permissions());
    commit(vec![
        change(source.clone(), Some(b"before"), None),
        addition,
    ])
    .unwrap();
    assert!(!source.exists());
    assert_eq!(fs::read(&target).unwrap(), b"after");
    assert_eq!(
        fs::metadata(target).unwrap().permissions().mode() & 0o777,
        0o750
    );
}
