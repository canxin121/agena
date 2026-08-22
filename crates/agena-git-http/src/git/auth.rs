use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
/// Authentication input for git network operations.
pub struct GitAuthInput {
    pub username: Option<String>,
    pub password: Option<String>,
}

pub(crate) fn normalize_http_auth(auth: &GitAuthInput) -> Option<(String, String)> {
    let username = auth.username.as_deref().unwrap_or("").trim().to_string();
    let password = auth.password.as_deref().unwrap_or("").trim().to_string();
    if username.is_empty() || password.is_empty() {
        return None;
    }
    Some((username, password))
}

fn git_http_auth_options(username: &str, password: &str) -> Vec<String> {
    // Avoid putting secrets directly into argv.
    // We still disable credential helpers so the operation is predictable.
    let _ = (username, password);
    vec!["-c".into(), "credential.helper=".into()]
}

pub(crate) struct TempGitAskpass {
    pub(crate) path: tempfile::TempPath,
}

async fn create_git_askpass_script() -> Result<TempGitAskpass, String> {
    // This script contains no secrets; it reads them from env vars.
    // `TempPath` provides collision-safe creation and automatic cleanup while
    // keeping credentials out of argv.
    let path = tempfile::Builder::new()
        .prefix("agena-git-http-askpass-")
        .suffix(".sh")
        .tempfile()
        .map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                "failed to create the temporary Git askpass script",
                &error,
            )
        })?
        .into_temp_path();

    let body = "#!/usr/bin/env sh\n\
set -e\n\
prompt=\"$1\"\n\
case \"$prompt\" in\n\
  *Username*|*username*) printf '%s' \"${OC_GIT_ASKPASS_USERNAME:-}\" ;;\n\
  *) printf '%s' \"${OC_GIT_ASKPASS_PASSWORD:-}\" ;;\n\
esac\n";

    tokio::fs::write(&*path, body).await.map_err(|error| {
        agena_failure::diagnostic::format_error_chain_with_context(
            "failed to write the temporary Git askpass script",
            &error,
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        tokio::fs::set_permissions(&*path, perms)
            .await
            .map_err(|error| {
                agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to secure the temporary Git askpass script",
                    &error,
                )
            })?;
    }

    Ok(TempGitAskpass { path })
}

pub(crate) async fn git_http_auth_env(
    username: &str,
    password: &str,
) -> Result<(Vec<String>, Vec<(String, String)>, TempGitAskpass), String> {
    let args = git_http_auth_options(username, password);
    let askpass = create_git_askpass_script().await?;
    let env = vec![
        (
            "GIT_ASKPASS".to_string(),
            askpass.path.to_string_lossy().into_owned(),
        ),
        (
            "OC_GIT_ASKPASS_USERNAME".to_string(),
            username.trim().to_string(),
        ),
        (
            "OC_GIT_ASKPASS_PASSWORD".to_string(),
            password.trim().to_string(),
        ),
    ];
    Ok((args, env, askpass))
}
