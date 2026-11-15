//! Reject policy features unavailable in Agena's current native adapter.

use super::{PermissionConfig, PermissionConfigError};

pub(super) fn validate(config: &PermissionConfig) -> Result<(), PermissionConfigError> {
    #[cfg(target_os = "windows")]
    {
        use agena_domain::{PathPermissionMode, PermissionMode};

        // Agena has not yet implemented an identity-bound proxy for PSEC.
        // Accepting these decisions would persist policies every launch rejects.
        if let Some(network) = &config.network {
            for (field, mode) in [
                ("permission.network.internet", network.internet),
                ("permission.network.private", network.private),
                ("permission.network.loopback", network.loopback),
            ] {
                if mode.is_some_and(|mode| mode != PermissionMode::Deny) {
                    return Err(unsupported_network(field));
                }
            }
            for (pattern, mode) in &network.rules {
                if *mode != PermissionMode::Deny {
                    return Err(unsupported_network(&format!(
                        "permission.network.rules[{pattern:?}]"
                    )));
                }
            }
        }

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

#[cfg(target_os = "windows")]
fn unsupported_network(field: &str) -> PermissionConfigError {
    PermissionConfigError::UnsupportedSandbox {
        field: field.into(),
        reason: "Agena's Windows MXC adapter currently supports offline execution only; set internet, private, loopback and network rules to deny".into(),
    }
}
