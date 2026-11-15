//! Filesystem path comparisons shared by configuration and runtime checks.
//! Windows follows the same ordinal, case-insensitive component comparison
//! used by the process sandbox. Command patterns do not use this module.

use std::path::{Component, Path, PathBuf};

pub(crate) fn equal(left: &Path, right: &Path) -> bool {
    let mut left = left.components();
    let mut right = right.components();
    loop {
        match (left.next(), right.next()) {
            (Some(left), Some(right)) if component_equal(left, right) => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

pub(crate) fn within(path: &Path, root: &Path) -> bool {
    if root.as_os_str().is_empty() {
        return false;
    }
    let mut path = path.components();
    root.components().all(|expected| {
        path.next()
            .is_some_and(|actual| component_equal(actual, expected))
    })
}

pub(crate) fn relative(path: &Path, root: &Path) -> Option<PathBuf> {
    within(path, root).then(|| {
        path.components()
            .skip(root.components().count())
            .map(|component| component.as_os_str())
            .collect()
    })
}

#[cfg(not(windows))]
fn component_equal(left: Component<'_>, right: Component<'_>) -> bool {
    left == right
}

#[cfg(windows)]
fn component_equal(left: Component<'_>, right: Component<'_>) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Prefix;
    use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};

    let wide = |component: Component<'_>| -> Vec<u16> {
        match component {
            Component::Prefix(prefix) => match prefix.kind() {
                // Canonicalize() adds the verbatim prefix. It does not change
                // which drive or resource the permission refers to.
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                    vec![u16::from(drive), u16::from(b':')]
                }
                _ => component.as_os_str().encode_wide().collect(),
            },
            Component::RootDir => vec![u16::from(b'\\')],
            _ => component.as_os_str().encode_wide().collect(),
        }
    };
    let left = wide(left);
    let right = wide(right);
    let (Ok(left_len), Ok(right_len)) = (i32::try_from(left.len()), i32::try_from(right.len()))
    else {
        return false;
    };
    // Both UTF-16 buffers are live for the call and their checked lengths
    // exclude a terminator. The API performs locale-independent comparison.
    unsafe {
        CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) == CSTR_EQUAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_boundaries_do_not_match_sibling_names() {
        assert!(within(
            Path::new("/workspace/output/file"),
            Path::new("/workspace/output")
        ));
        assert!(!within(
            Path::new("/workspace/output-other/file"),
            Path::new("/workspace/output")
        ));
        assert_eq!(
            relative(
                Path::new("/workspace/output/file"),
                Path::new("/workspace/output")
            ),
            Some(PathBuf::from("file"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_context_rejects_case_equivalent_future_boundaries() {
        let workspace =
            std::env::temp_dir().join(format!("agena-ordinal-policy-{}", std::process::id()));
        let config: agena_domain::PermissionConfig = serde_json::from_value(serde_json::json!({
            "path": {
                "workspace":"deny", "external":"deny",
                "rules":{"<workspace>/ÉTUDES":"deny", "<workspace>/études/new.txt":"readwrite"}
            }
        }))
        .unwrap();
        let error =
            crate::authorization::validate_permission_config_for_workspace(&config, &workspace)
                .unwrap_err()
                .to_string();
        assert!(error.contains("conflicts"), "{error}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_literal_subtree_and_snapshot_matching_keep_unicode_denials() {
        use crate::authorization::ExecutionPrincipal;
        use crate::permission::{PermissionPolicy, ToolPermissionPolicy, WildcardPattern};
        use agena_domain::{AccessKind, PermissionDecision};
        let workspace = Path::new(r"C:\AgenaTest\ÉTUDES");
        let target = Path::new(r"\\?\c:\agenatest\études\sécret\future.txt");
        let config = serde_json::from_value(serde_json::json!({
            "path": {"workspace":"readwrite", "external":"deny", "rules":{"<workspace>/SÉCRET":"deny"}}
        })).unwrap();
        let principal = ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(&config)
        .unwrap();
        assert!(matches!(
            principal.authorize_path_access(AccessKind::Read, workspace, target),
            PermissionDecision::Deny { .. }
        ));
        assert!(matches!(
            principal
                .sandbox_permission_snapshot(workspace)
                .authorize_path_access(AccessKind::Read, target),
            PermissionDecision::Deny { .. }
        ));
        // The path-specific correction does not alter shell command matching.
        assert!(!WildcardPattern::new("echo É").matches("echo é"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_ordinal_identity_handles_unicode_and_verbatim_drive_prefixes() {
        assert!(equal(
            Path::new(r"C:\ÉTUDES\result.txt"),
            Path::new(r"\\?\c:\études\RESULT.TXT")
        ));
        assert!(within(
            Path::new(r"C:\études\future\file"),
            Path::new(r"\\?\c:\ÉTUDES")
        ));
        assert_eq!(
            relative(
                Path::new(r"C:\études\future\file"),
                Path::new(r"\\?\c:\ÉTUDES")
            ),
            Some(PathBuf::from(r"future\file"))
        );
    }
}
