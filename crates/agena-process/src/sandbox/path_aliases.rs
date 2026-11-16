//! Preserve native path identities and verified macOS Data-volume aliases.
//!
//! A firmlink can expose one inode through two absolute paths without ordinary
//! canonicalization equating them. System-volume paths must not be mirrored
//! merely because a similarly named path exists on the Data volume.

use super::policy_path_identity;
#[cfg(any(target_os = "macos", test))]
use std::path::{Component, Path};
use std::{io, path::PathBuf};

pub(super) fn identities(paths: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    let mut values = Vec::new();
    for path in paths {
        let canonical = policy_path_identity(path)?;
        values.push(path.clone());
        values.push(canonical.clone());
        #[cfg(target_os = "macos")]
        for spelling in [path.as_path(), canonical.as_path()] {
            if let Some(alias) = confirmed_data_alias(spelling, file_identity)? {
                values.push(alias);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let table = match std::fs::read_to_string("/usr/share/firmlinks") {
            Ok(table) => table,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error),
        };
        let partial = confirmed_firmlink_aliases(&values, &table, file_identity)?;
        values.extend(partial);
    }
    values.sort();
    values.dedup();
    Ok(values)
}

#[cfg(any(target_os = "macos", test))]
const DATA_ROOT: &str = "/System/Volumes/Data";

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

/// Produce only a candidate. Its filesystem identity must be confirmed before
/// it can participate in either a grant or a restriction.
#[cfg(any(target_os = "macos", test))]
fn data_counterpart(path: &Path) -> Option<PathBuf> {
    let root = Path::new("/");
    let data = Path::new(DATA_ROOT);
    // Treat these as macOS paths even in tests hosted on another OS.
    if !path.starts_with(root)
        || path == root
        || path == data
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return None;
    }
    if let Ok(relative) = path.strip_prefix(data) {
        Some(root.join(relative))
    } else {
        path.strip_prefix(root)
            .ok()
            .map(|relative| data.join(relative))
    }
}

/// Missing tails are equivalent only when their nearest existing ancestors
/// identify the same object. One-sided existence or a different existing inode
/// stops the search, rather than falling back to a more permissive ancestor.
#[cfg(any(target_os = "macos", test))]
fn confirmed_data_alias<F>(path: &Path, mut lookup: F) -> io::Result<Option<PathBuf>>
where
    F: FnMut(&Path) -> io::Result<Option<FileIdentity>>,
{
    let Some(alias) = data_counterpart(path) else {
        return Ok(None);
    };
    let mut source = path;
    let mut target = alias.as_path();
    loop {
        // The System and Data roots themselves are never interchangeable.
        // A missing child cannot establish an alias by reaching those roots.
        if [source, target]
            .iter()
            .any(|path| *path == Path::new("/") || *path == Path::new(DATA_ROOT))
        {
            return Ok(None);
        }
        match (lookup(source)?, lookup(target)?) {
            (Some(source_id), Some(target_id)) => {
                return Ok((source_id == target_id).then_some(alias));
            }
            (None, None) => {
                let (Some(source_parent), Some(target_parent)) = (source.parent(), target.parent())
                else {
                    return Ok(None);
                };
                source = source_parent;
                target = target_parent;
            }
            _ => return Ok(None),
        }
    }
}

/// The system table identifies candidate subtrees; it does not itself grant
/// anything. Normalize sources only and verify their Data-view identity below.
#[cfg(any(target_os = "macos", test))]
fn firmlink_sources(table: &str) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    for line in table.lines() {
        let Some(source) = line.split_whitespace().next() else {
            continue;
        };
        if source.starts_with('#')
            || source.chars().any(char::is_control)
            || source.split('/').any(|part| part == "..")
        {
            continue;
        }
        let parts: Vec<_> = source
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect();
        let source = PathBuf::from(format!("/{}", parts.join("/")));
        if data_counterpart(&source).is_some() {
            sources.push(source);
        }
    }
    sources.sort();
    sources.dedup();
    sources
}

/// A System-volume ancestor can contain individual Data-volume firmlinks even
/// when the two ancestor directories are different objects. Project only those
/// table roots fully contained in an input scope, without traversing the tree.
#[cfg(any(target_os = "macos", test))]
fn confirmed_firmlink_aliases<F>(
    paths: &[PathBuf],
    table: &str,
    mut lookup: F,
) -> io::Result<Vec<PathBuf>>
where
    F: FnMut(&Path) -> io::Result<Option<FileIdentity>>,
{
    let mut values = Vec::new();
    for source in firmlink_sources(table) {
        let Some(candidate) = data_counterpart(&source) else {
            continue;
        };
        if !paths
            .iter()
            .any(|root| source.starts_with(root) || candidate.starts_with(root))
        {
            continue;
        }
        if let Some(alias) = confirmed_data_alias(&source, &mut lookup)? {
            values.extend([source, alias]);
        }
    }
    values.sort();
    values.dedup();
    Ok(values)
}

#[cfg(target_os = "macos")]
fn file_identity(path: &Path) -> io::Result<Option<FileIdentity>> {
    use std::os::unix::fs::MetadataExt;

    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Do not classify an existing dangling symlink as a future path:
            // relative symlinks can resolve differently through the two views.
            match std::fs::symlink_metadata(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "cannot confirm sandbox Data-volume alias through unresolved path {}",
                        path.display()
                    ),
                )),
            }
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn id(device: u64, inode: u64) -> FileIdentity {
        FileIdentity { device, inode }
    }

    fn aliases(path: &str, entries: &[(&str, FileIdentity)]) -> io::Result<Option<PathBuf>> {
        let entries: BTreeMap<_, _> = entries
            .iter()
            .map(|(path, identity)| (PathBuf::from(*path), *identity))
            .collect();
        confirmed_data_alias(Path::new(path), |path| Ok(entries.get(path).copied()))
    }

    #[test]
    fn candidate_mapping_is_bidirectional_and_does_not_repeat_data_prefix() {
        assert_eq!(
            data_counterpart(Path::new("/Users/alice/tool")),
            Some(PathBuf::from("/System/Volumes/Data/Users/alice/tool"))
        );
        assert_eq!(
            data_counterpart(Path::new("/System/Volumes/Data/Users/alice/tool")),
            Some(PathBuf::from("/Users/alice/tool"))
        );
        for path in ["/", DATA_ROOT, "relative/tool", "/Users/../System"] {
            assert_eq!(data_counterpart(Path::new(path)), None, "{path}");
        }
    }

    #[test]
    fn matching_device_and_inode_confirm_an_existing_alias_in_both_directions() {
        let logical = "/Users/alice/bin/agena";
        let physical = "/System/Volumes/Data/Users/alice/bin/agena";
        let entries = [(logical, id(7, 42)), (physical, id(7, 42))];
        assert_eq!(aliases(logical, &entries).unwrap(), Some(physical.into()));
        assert_eq!(aliases(physical, &entries).unwrap(), Some(logical.into()));
    }

    #[test]
    fn matching_inode_on_another_device_does_not_establish_an_alias() {
        assert_eq!(
            aliases(
                "/Users/alice",
                &[
                    ("/Users/alice", id(1, 42)),
                    ("/System/Volumes/Data/Users/alice", id(2, 42)),
                ]
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn distinct_system_and_data_directories_are_not_mirrored() {
        assert_eq!(
            aliases(
                "/System/Library",
                &[
                    ("/System/Library", id(1, 10)),
                    ("/System/Volumes/Data/System/Library", id(2, 20)),
                ]
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn future_deny_descendants_use_a_confirmed_existing_ancestor() {
        let source = "/Users/alice/future/secrets";
        let target = "/System/Volumes/Data/Users/alice/future/secrets";
        let entries = [
            ("/Users/alice", id(7, 42)),
            ("/System/Volumes/Data/Users/alice", id(7, 42)),
        ];
        assert_eq!(aliases(source, &entries).unwrap(), Some(target.into()));
        assert_eq!(aliases(target, &entries).unwrap(), Some(source.into()));
    }

    #[test]
    fn one_sided_existence_is_not_replaced_by_an_equal_parent() {
        let entries = [
            ("/Users/alice/future", id(7, 99)),
            ("/Users/alice", id(7, 42)),
            ("/System/Volumes/Data/Users/alice", id(7, 42)),
        ];
        assert_eq!(aliases("/Users/alice/future", &entries).unwrap(), None);
    }

    #[test]
    fn missing_ancestors_cannot_be_authorized_by_equating_volume_roots() {
        let entries = [("/", id(7, 1)), (DATA_ROOT, id(7, 1))];
        assert_eq!(aliases("/unconfirmed/future", &entries).unwrap(), None);
    }

    #[test]
    fn identity_lookup_errors_do_not_become_missing_ancestors() {
        let error = confirmed_data_alias(Path::new("/Users/alice/private"), |_| {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "fixture"))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn firmlink_sources_normalize_relative_paths_and_reject_parent_traversal() {
        assert_eq!(
            firmlink_sources(
                "# system table\n\nusr/local\tusr/local\n/Users Users\n/usr//local usr/local\n/usr/../etc etc\n../private private\n/ /\n"
            ),
            [PathBuf::from("/Users"), PathBuf::from("/usr/local")]
        );
    }

    #[test]
    fn partial_firmlink_projects_a_confirmed_child_in_both_parent_views() {
        let entries: BTreeMap<_, _> = [
            (PathBuf::from("/usr"), id(1, 10)),
            (PathBuf::from("/System/Volumes/Data/usr"), id(2, 20)),
            (PathBuf::from("/usr/local"), id(2, 30)),
            (PathBuf::from("/System/Volumes/Data/usr/local"), id(2, 30)),
            (PathBuf::from("/usr/share"), id(1, 40)),
            (PathBuf::from("/System/Volumes/Data/usr/share"), id(2, 50)),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            confirmed_data_alias(Path::new("/usr"), |path| Ok(entries.get(path).copied())).unwrap(),
            None
        );
        let mut expected = vec![
            PathBuf::from("/usr/local"),
            PathBuf::from("/System/Volumes/Data/usr/local"),
        ];
        expected.sort();
        for parent in ["/usr", "/System/Volumes/Data/usr"] {
            let actual = confirmed_firmlink_aliases(
                &[PathBuf::from(parent)],
                "usr/local usr/local\nusr/share usr/share\nUsers Users\n",
                |path| {
                    assert!(!path.starts_with("/Users"));
                    Ok(entries.get(path).copied())
                },
            )
            .unwrap();
            assert_eq!(actual, expected);
            assert!(!actual.contains(&PathBuf::from("/usr")));
            assert!(!actual.contains(&PathBuf::from("/System/Volumes/Data/usr")));
        }
    }

    #[test]
    fn descendant_scope_does_not_gain_the_entire_firmlink_parent() {
        let paths = [PathBuf::from("/usr/local/project")];
        assert!(
            confirmed_firmlink_aliases(&paths, "usr/local usr/local\n", |_| {
                panic!("an ancestor outside the input scope must not be projected")
            })
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn empty_legacy_table_and_unconfirmed_entries_add_no_roots() {
        let paths = [PathBuf::from("/usr")];
        assert!(
            confirmed_firmlink_aliases(&paths, "", |_| {
                panic!("an empty legacy table needs no identity queries")
            })
            .unwrap()
            .is_empty()
        );
        assert!(
            confirmed_firmlink_aliases(&paths, "usr/local usr/local\n", |path| {
                Ok((path == Path::new("/usr/local")).then_some(id(2, 30)))
            })
            .unwrap()
            .is_empty()
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn non_mac_identity_preserves_symlink_spelling_and_canonical_target() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let link = directory.path().join("link");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let values = identities(&[link.clone()]).unwrap();
        assert!(values.contains(&link));
        assert!(values.contains(&target.canonicalize().unwrap()));
        assert_eq!(values.len(), 2);
    }
}
