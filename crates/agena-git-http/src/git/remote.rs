use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::utils::git_config_get;
use super::{
    DirectoryQuery, git_command_result_or_log, git_io_error_response, git_success_response,
    require_directory, require_directory_raw, run_git, run_git_checked, run_git_env,
    run_locked_git_checked,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// Information about a git remote.
pub struct GitRemoteInfo {
    pub name: String,
    pub url: String,
    pub protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// Response listing git remotes.
pub struct GitRemoteInfoResponse {
    pub remotes: Vec<GitRemoteInfo>,
}

fn parse_remote_url(url: &str) -> (String, Option<String>) {
    let u = url.trim();
    if let Ok(parsed) = url::Url::parse(u) {
        let protocol = match parsed.scheme() {
            "http" => "http",
            "https" => "https",
            "ssh" | "git+ssh" => "ssh",
            "file" => "file",
            _ => "unknown",
        };
        let host = parsed.host_str().map(str::to_string);
        return (protocol.to_string(), host);
    }

    // Git also accepts an scp-like syntax that is intentionally not a URL.
    if let Some((authority, path)) = u.split_once(':')
        && !authority.contains(['/', '\\'])
        && !path.is_empty()
    {
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let host = host.trim();
        return (
            "ssh".to_string(),
            if host.is_empty() {
                None
            } else {
                Some(host.to_string())
            },
        );
    }
    // local path or unknown.
    ("unknown".to_string(), None)
}

pub async fn git_remote_info(Query(q): Query<DirectoryQuery>) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let (out, _err) =
        match run_git_checked(&dir, &["remote", "-v"], Some("git_remote_failed")).await {
            Ok(value) => value,
            Err(resp) => return resp,
        };

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut remotes: Vec<GitRemoteInfo> = Vec::new();
    for line in out.lines() {
        // format: <name> <url> (fetch)
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let name = parts.next().unwrap_or("").trim();
        let url = parts.next().unwrap_or("").trim();
        if name.is_empty() || url.is_empty() {
            continue;
        }
        let key = (name.to_string(), url.to_string());
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        let (protocol, host) = parse_remote_url(url);
        remotes.push(GitRemoteInfo {
            name: name.to_string(),
            url: url.to_string(),
            protocol,
            host,
        });
    }
    remotes.sort_by(|a, b| a.name.cmp(&b.name).then(a.url.cmp(&b.url)));

    Json(GitRemoteInfoResponse { remotes }).into_response()
}

#[derive(Debug, Deserialize)]
/// Body of a git remote add request.
pub struct GitRemoteAddBody {
    pub name: Option<String>,
    pub url: Option<String>,
}

pub async fn git_remote_add(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitRemoteAddBody>,
) -> Response {
    let Some(name) = body
        .name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "name is required", "code": "missing_name"})),
        )
            .into_response();
    };
    let Some(url) = body
        .url
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "url is required", "code": "missing_url"})),
        )
            .into_response();
    };

    if let Err(resp) = run_locked_git_checked(
        &q,
        &["remote", "add", name, url],
        Some("git_remote_add_failed"),
    )
    .await
    {
        return resp;
    }

    git_success_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Body of a git remote rename request.
pub struct GitRemoteRenameBody {
    pub name: Option<String>,
    pub new_name: Option<String>,
}

pub async fn git_remote_rename(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitRemoteRenameBody>,
) -> Response {
    let Some(name) = body
        .name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "name is required", "code": "missing_name"})),
        )
            .into_response();
    };
    let Some(new_name) = body
        .new_name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "newName is required", "code": "missing_new_name"})),
        )
            .into_response();
    };

    if let Err(resp) = run_locked_git_checked(
        &q,
        &["remote", "rename", name, new_name],
        Some("git_remote_rename_failed"),
    )
    .await
    {
        return resp;
    }

    git_success_response()
}

#[derive(Debug, Deserialize)]
/// Body of a git remote set-url request.
pub struct GitRemoteSetUrlBody {
    pub name: Option<String>,
    pub url: Option<String>,
}

pub async fn git_remote_set_url(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitRemoteSetUrlBody>,
) -> Response {
    let Some(name) = body
        .name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "name is required", "code": "missing_name"})),
        )
            .into_response();
    };
    let Some(url) = body
        .url
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "url is required", "code": "missing_url"})),
        )
            .into_response();
    };

    if let Err(resp) = run_locked_git_checked(
        &q,
        &["remote", "set-url", name, url],
        Some("git_remote_set_url_failed"),
    )
    .await
    {
        return resp;
    }

    git_success_response()
}

#[derive(Debug, Deserialize)]
/// Body of a git remote remove request.
pub struct GitRemoteRemoveBody {
    pub name: Option<String>,
}

pub async fn git_remote_remove(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitRemoteRemoveBody>,
) -> Response {
    let Some(name) = body
        .name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "name is required", "code": "missing_name"})),
        )
            .into_response();
    };

    if let Err(resp) = run_locked_git_checked(
        &q,
        &["remote", "remove", name],
        Some("git_remote_remove_failed"),
    )
    .await
    {
        return resp;
    }

    git_success_response()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// Response with git signing information.
pub struct GitSigningInfoResponse {
    pub commit_gpgsign: bool,
    pub gpg_format: String,
    pub signing_key: Option<String>,
    pub gpg_program: Option<String>,

    // SSH signing (when gpg.format=ssh).
    pub ssh_signing_key: Option<String>,
    pub ssh_auth_sock_present: bool,
    pub ssh_agent_has_keys: bool,
    pub ssh_signing_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_agent_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_signing_probe_error: Option<String>,
}

pub async fn git_signing_info(Query(q): Query<DirectoryQuery>) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let global_commit_gpgsign = git_config_get(None, "--global", "commit.gpgsign").await;
    let raw_commit_gpgsign = git_config_get(Some(&dir), "--local", "commit.gpgsign")
        .await
        .or(global_commit_gpgsign)
        .unwrap_or_else(|| "false".to_string())
        .to_ascii_lowercase();

    let commit_gpgsign =
        raw_commit_gpgsign == "true" || raw_commit_gpgsign == "1" || raw_commit_gpgsign == "yes";

    let global_gpg_format = git_config_get(None, "--global", "gpg.format").await;
    let gpg_format = git_config_get(Some(&dir), "--local", "gpg.format")
        .await
        .or(global_gpg_format)
        .unwrap_or_else(|| "openpgp".to_string());

    let global_signing_key = git_config_get(None, "--global", "user.signingkey").await;
    let signing_key = git_config_get(Some(&dir), "--local", "user.signingkey")
        .await
        .or(global_signing_key);

    let global_gpg_program = git_config_get(None, "--global", "gpg.program").await;
    let gpg_program = git_config_get(Some(&dir), "--local", "gpg.program")
        .await
        .or(global_gpg_program);

    let global_ssh_signing_key = git_config_get(None, "--global", "ssh.signingkey").await;
    let ssh_signing_key = git_config_get(Some(&dir), "--local", "ssh.signingkey")
        .await
        .or(global_ssh_signing_key);

    let (ssh_auth_sock_present, ssh_agent_has_keys, ssh_agent_error) = ssh_agent_probe().await;

    let probe_result = if commit_gpgsign && gpg_format.trim().eq_ignore_ascii_case("ssh") {
        Some(
            ssh_signing_probe(
                &gpg_format,
                signing_key.as_deref(),
                gpg_program.as_deref(),
                ssh_signing_key.as_deref(),
            )
            .await,
        )
    } else {
        None
    };

    let (ssh_signing_available, ssh_signing_probe_error) =
        resolve_ssh_signing_status(ssh_auth_sock_present, ssh_agent_has_keys, probe_result);

    Json(GitSigningInfoResponse {
        commit_gpgsign,
        gpg_format: gpg_format.trim().to_string(),
        signing_key,
        gpg_program,
        ssh_signing_key,
        ssh_auth_sock_present,
        ssh_agent_has_keys,
        ssh_signing_available,
        ssh_agent_error,
        ssh_signing_probe_error,
    })
    .into_response()
}

fn resolve_ssh_signing_status(
    ssh_auth_sock_present: bool,
    ssh_agent_has_keys: bool,
    probe_result: Option<(bool, Option<String>)>,
) -> (bool, Option<String>) {
    if let Some((available, error)) = probe_result {
        return (available, error);
    }
    (ssh_auth_sock_present && ssh_agent_has_keys, None)
}

async fn ssh_signing_probe(
    gpg_format: &str,
    signing_key: Option<&str>,
    gpg_program: Option<&str>,
    ssh_signing_key: Option<&str>,
) -> (bool, Option<String>) {
    let temp_dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => {
            return (
                false,
                Some(format!("unable to create probe repository: {err}")),
            );
        }
    };

    let (init_code, _init_out, init_err) = match run_git(temp_dir.path(), &["init", "-q"]).await {
        Ok(result) => result,
        Err(error) => {
            return (
                false,
                Some(format!(
                    "SSH signing probe Git initialization failed: {error}"
                )),
            );
        }
    };
    if init_code != 0 {
        let msg = init_err.trim();
        if msg.is_empty() {
            return (
                false,
                Some("git init failed during SSH signing probe".to_string()),
            );
        }
        return (false, Some(msg.to_string()));
    }

    let mut args = vec![
        "-c".to_string(),
        "user.name=Agena SSH Probe".to_string(),
        "-c".to_string(),
        "user.email=agena-ssh-probe@example.invalid".to_string(),
        "-c".to_string(),
        "commit.gpgsign=true".to_string(),
        "-c".to_string(),
        format!("gpg.format={}", gpg_format.trim()),
    ];

    if let Some(v) = signing_key.map(str::trim).filter(|v| !v.is_empty()) {
        args.push("-c".to_string());
        args.push(format!("user.signingkey={v}"));
    }
    if let Some(v) = gpg_program.map(str::trim).filter(|v| !v.is_empty()) {
        args.push("-c".to_string());
        args.push(format!("gpg.program={v}"));
    }
    if let Some(v) = ssh_signing_key.map(str::trim).filter(|v| !v.is_empty()) {
        args.push("-c".to_string());
        args.push(format!("ssh.signingkey={v}"));
    }

    args.push("commit".to_string());
    args.push("--allow-empty".to_string());
    args.push("--no-verify".to_string());
    args.push("-S".to_string());
    args.push("-m".to_string());
    args.push("agena ssh signing probe".to_string());

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let env = [("LC_ALL", "C")];

    let (code, out, err) = match run_git_env(temp_dir.path(), &arg_refs, &env).await {
        Ok(result) => result,
        Err(error) => {
            return (
                false,
                Some(format!("SSH signing probe process failed: {error}")),
            );
        }
    };

    if code == 0 {
        return (true, None);
    }

    let msg = err
        .lines()
        .chain(out.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
        .unwrap_or_else(|| "SSH signing probe failed".to_string());
    (false, Some(msg))
}

async fn ssh_agent_probe() -> (bool, bool, Option<String>) {
    // Heuristic only: VS Code delegates to environment/agent. We do the same and
    // return enough info for the UI to guide users.
    let sock = std::env::var("SSH_AUTH_SOCK").unwrap_or_default();
    if sock.trim().is_empty() {
        return (false, false, None);
    }

    // `ssh-add -L` prints public keys; it should not require interaction.
    let mut cmd = Command::new("ssh-add");
    cmd.args(["-L"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let Ok(output) = agena_process::output(cmd, Duration::from_secs(2), 1024 * 1024).await else {
        return (
            true,
            false,
            Some("ssh-add probe timed out or failed".to_string()),
        );
    };

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if err.is_empty() {
            return (true, false, Some("ssh-add returned an error".to_string()));
        }
        return (true, false, Some(err));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let has_keys = stdout.lines().any(|l| !l.trim().is_empty());
    (true, has_keys, None)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// State of a git repository.
pub struct GitRepoStateResponse {
    pub current_branch: Option<String>,
    pub upstream: Option<String>,
    pub merge_in_progress: bool,
    pub rebase_in_progress: bool,
    pub cherry_pick_in_progress: bool,
    pub revert_in_progress: bool,
}

async fn git_path_exists(dir: &Path, name: &str) -> Result<bool, Response> {
    let (code, out, error) = match run_git(dir, &["rev-parse", "--git-path", name]).await {
        Ok(result) => result,
        Err(error) => {
            return Err(super::git_command_transport_error_response(
                "resolve Git repository state marker",
                &error,
                Some("git_state_process_failed"),
            ));
        }
    };
    if code != 0 {
        if let Some(response) = super::map_git_failure(code, &out, &error) {
            return Err(response);
        }
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": error.trim(),
                "code": "git_state_marker_failed",
            })),
        )
            .into_response());
    }
    let raw = out.trim();
    if raw.is_empty() {
        return Ok(false);
    }
    let p = PathBuf::from(raw);
    let full = if p.is_absolute() { p } else { dir.join(p) };
    match tokio::fs::metadata(full).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(git_io_error_response(
            "inspect Git repository state marker",
            &error,
            "git_state_marker_metadata_failed",
        )),
    }
}

pub async fn git_state(Query(q): Query<DirectoryQuery>) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let current_branch = git_current_branch(&dir).await;
    let upstream = git_upstream_ref(&dir).await;

    let merge_in_progress = match git_path_exists(&dir, "MERGE_HEAD").await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let cherry_pick_in_progress = match git_path_exists(&dir, "CHERRY_PICK_HEAD").await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let revert_in_progress = match git_path_exists(&dir, "REVERT_HEAD").await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let rebase_apply = match git_path_exists(&dir, "rebase-apply").await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let rebase_merge = match git_path_exists(&dir, "rebase-merge").await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let rebase_in_progress = rebase_apply || rebase_merge;

    Json(GitRepoStateResponse {
        current_branch,
        upstream,
        merge_in_progress,
        rebase_in_progress,
        cherry_pick_in_progress,
        revert_in_progress,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
/// Query for remote branches.
pub struct GitRemoteBranchesQuery {
    pub directory: Option<String>,
    pub remote: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// Response listing remote branches.
pub struct GitRemoteBranchListResponse {
    pub remote: String,
    pub branches: Vec<String>,
}

pub async fn git_remote_branches_list(Query(q): Query<GitRemoteBranchesQuery>) -> Response {
    let dir = match require_directory_raw(q.directory.as_deref()) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };
    let remote = q
        .remote
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("origin");

    // Use ls-remote for remote heads without fetching.
    let (out, _err) = match run_git_checked(&dir, &["ls-remote", "--heads", remote], None).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let mut branches: Vec<String> = Vec::new();
    for line in out.lines() {
        // <hash>\trefs/heads/<name>
        let Some((_hash, r)) = line.split_once('\t') else {
            continue;
        };
        let r = r.trim();
        if let Some(name) = r.strip_prefix("refs/heads/") {
            let n = name.trim();
            if !n.is_empty() {
                branches.push(n.to_string());
            }
        }
    }
    branches.sort();
    branches.dedup();

    Json(GitRemoteBranchListResponse {
        remote: remote.to_string(),
        branches,
    })
    .into_response()
}

pub(crate) async fn git_current_branch(dir: &Path) -> Option<String> {
    // `symbolic-ref` returns a stable answer and fails on detached HEAD.
    let (code, out, _) = git_command_result_or_log(
        run_git(dir, &["symbolic-ref", "--short", "HEAD"]).await,
        "resolve current Git branch",
    )?;
    if code != 0 {
        return None;
    }
    let b = out.trim();
    if b.is_empty() {
        None
    } else {
        Some(b.to_string())
    }
}

pub(crate) async fn git_upstream_ref(dir: &Path) -> Option<String> {
    // Examples: "origin/main". Fails if no upstream is configured.
    let (code, out, _) = git_command_result_or_log(
        run_git(
            dir,
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
        )
        .await,
        "resolve current Git upstream",
    )?;
    if code != 0 {
        return None;
    }
    let s = out.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::parse_remote_url;

    #[test]
    fn remote_urls_use_url_parser_and_keep_git_scp_syntax() {
        assert_eq!(
            parse_remote_url("http://example.com:8080/team/repo.git"),
            ("http".to_string(), Some("example.com".to_string()))
        );
        assert_eq!(
            parse_remote_url("ssh://git@[2001:db8::1]/team/repo.git"),
            ("ssh".to_string(), Some("[2001:db8::1]".to_string()))
        );
        assert_eq!(
            parse_remote_url("git@example.com:team/repo.git"),
            ("ssh".to_string(), Some("example.com".to_string()))
        );
        assert_eq!(
            parse_remote_url("../local/repo.git"),
            ("unknown".to_string(), None)
        );
    }
}
