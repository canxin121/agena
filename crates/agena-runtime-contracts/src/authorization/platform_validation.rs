//! Reject policy features unavailable in Agena's current native adapter.

use super::{PermissionConfig, PermissionConfigError};

pub(super) fn validate(config: &PermissionConfig) -> Result<(), PermissionConfigError> {
    if let Some(network) = &config.network {
        network
            .validate_for_platform(std::env::consts::OS)
            .map_err(|reason| PermissionConfigError::UnsupportedSandbox {
                field: "permission.network".into(),
                reason,
            })?;
    }
    #[cfg(target_os = "windows")]
    {
        use agena_domain::PathPermissionMode;

        if let Some(paths) = &config.path {
            let has_explicit_deny = paths
                .rules
                .values()
                .any(|modes| modes.sandbox_mode() == Ok(PathPermissionMode::Deny));
            if has_explicit_deny
                && !mxc_sdk::process_container_common::base_container_runner::supports_native_denied_paths()
            {
                return Err(PermissionConfigError::UnsupportedSandbox {
                    field: "permission.path.rules".into(),
                    reason: "this Windows PSEC does not support FileSystemDeny; use default-deny with explicit allowed paths, or upgrade Windows before adding explicit deny paths".into(),
                });
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = config;
    Ok(())
}
