//! Short-lived, run-bound preview origins. Workbench credentials never reach apps.
use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use url::Url;

use super::registry::{PreviewSessionRecord, preview_target_from_record};
use crate::{ApiResult, AppError};

const GRANT_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_GRANTS: usize = 512;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreviewAccessRequest {
    pub(crate) backend_origin: String,
    pub(crate) workbench_origin: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviewAccessResponse {
    pub(crate) url: String,
    pub(crate) expires_at: i64,
    pub(crate) generation: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PreviewGrant {
    pub(crate) preview_id: String,
    pub(crate) generation: String,
    pub(crate) origin: Url,
    pub(crate) workbench_origin: String,
    pub(crate) expires_at: i64,
    /// Agent previews additionally enforce same-preview connection/child policy.
    pub(crate) browser_only: bool,
}

#[derive(Debug)]
pub(crate) struct PreviewAccess {
    configured_origin: Option<Url>,
    control_port: u16,
    grants: RwLock<HashMap<String, PreviewGrant>>,
}

pub(crate) fn parse_origin(raw: &str) -> ApiResult<Url> {
    let mut url = Url::parse(raw).map_err(|_| AppError::bad_request("invalid preview origin"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::bad_request(
            "preview origins must be HTTP(S) origins without credentials, paths, queries, or fragments",
        ));
    }
    // DNS treats a trailing dot as the same host. Keep configuration, minted
    // grants, request dispatch, and lease lookup on the same canonical spelling.
    if let Some(domain) = url.domain() {
        let domain = domain.trim_end_matches('.').to_owned();
        if domain.is_empty() {
            return Err(AppError::bad_request("invalid preview hostname"));
        }
        url.set_host(Some(&domain))
            .map_err(|_| AppError::bad_request("invalid preview hostname"))?;
    }
    Ok(url)
}

pub(crate) fn run_generation(record: &PreviewSessionRecord) -> String {
    // Bind access to launch identity and state, not a reusable PID alone.
    let identity = serde_json::json!([
        record.id,
        record.instance_id,
        record.directory,
        record.run_directory,
        record.agena_session_id,
        record.target_url,
        record.command,
        record.args,
        record.pid,
        record.started_at,
        record.state
    ]);
    Sha256::digest(identity.to_string().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl PreviewAccess {
    pub(crate) fn from_env(control_port: u16) -> ApiResult<Self> {
        Self::new(
            std::env::var("AGENA_PREVIEW_ORIGIN").ok().as_deref(),
            control_port,
        )
    }

    pub(crate) fn new(configured_origin: Option<&str>, control_port: u16) -> ApiResult<Self> {
        let configured_origin = configured_origin
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(parse_origin)
            .transpose()?;
        if let Some(origin) = &configured_origin
            && origin.domain().is_none()
        {
            return Err(AppError::bad_request(
                "AGENA_PREVIEW_ORIGIN must use a DNS hostname supporting wildcard subdomains",
            ));
        }
        Ok(Self {
            configured_origin,
            control_port,
            grants: RwLock::new(HashMap::new()),
        })
    }

    pub(crate) fn validate_upstream(&self, target: &Url) -> ApiResult<()> {
        if !target.username().is_empty() || target.password().is_some() {
            return Err(AppError::bad_request(
                "preview upstream URLs cannot contain credentials; use the application's authentication flow",
            ));
        }
        // Target validation already restricts upstreams to loopback. Compare
        // the real bound listener port, never an untrusted Host or public URL.
        if target.port_or_known_default() == Some(self.control_port) {
            return Err(AppError::forbidden(
                "a preview cannot proxy the Agena control listener",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_record(&self, record: &PreviewSessionRecord) -> ApiResult<Url> {
        let target = preview_target_from_record(record)?;
        self.validate_upstream(&target)?;
        Ok(target)
    }

    fn base_origin(&self, backend: &Url) -> ApiResult<Url> {
        if let Some(origin) = &self.configured_origin {
            return Ok(origin.clone());
        }
        match backend.host_str() {
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1") => {
                let mut origin = backend.clone();
                origin
                    .set_host(Some("localhost"))
                    .map_err(|_| AppError::bad_request("invalid local preview origin"))?;
                Ok(origin)
            }
            _ => Err(AppError::conflict(
                "Remote preview requires AGENA_PREVIEW_ORIGIN (for example https://preview.example.com) and wildcard DNS/TLS routing to this Agena server. The workbench origin is never used as an unsafe fallback.",
            )),
        }
    }

    // Unknown/expired preview hosts must never fall through to the control API.
    pub(crate) fn is_preview_host(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if host.starts_with("p-") && host.ends_with(".localhost") {
            return true;
        }
        self.configured_origin
            .as_ref()
            .and_then(Url::host_str)
            .is_some_and(|base| host != base && host.ends_with(&format!(".{base}")))
    }

    pub(crate) fn is_preview_origin(&self, raw: &str) -> bool {
        Url::parse(raw)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .is_some_and(|host| self.is_preview_host(&host))
    }

    pub(crate) async fn issue(
        &self,
        record: &PreviewSessionRecord,
        backend_origin: &str,
        workbench_origin: &str,
    ) -> ApiResult<PreviewAccessResponse> {
        self.issue_with_policy(record, backend_origin, workbench_origin, false)
            .await
    }

    pub(crate) async fn issue_for_browser(
        &self,
        record: &PreviewSessionRecord,
        backend_origin: &str,
    ) -> ApiResult<PreviewAccessResponse> {
        self.issue_with_policy(record, backend_origin, backend_origin, true)
            .await
    }

    async fn issue_with_policy(
        &self,
        record: &PreviewSessionRecord,
        backend_origin: &str,
        workbench_origin: &str,
        browser_only: bool,
    ) -> ApiResult<PreviewAccessResponse> {
        self.validate_record(record)?;
        let backend = parse_origin(backend_origin)?;
        let workbench = parse_origin(workbench_origin)?;
        if self.is_preview_origin(workbench_origin) {
            return Err(AppError::forbidden(
                "a preview cannot authorize another preview",
            ));
        }
        let base = self.base_origin(&backend)?;
        let generation = run_generation(record);
        let now = chrono::Utc::now().timestamp_millis();
        let expires_at = now + GRANT_TTL.as_millis() as i64;
        let workbench_origin = workbench.origin().ascii_serialization();
        let mut grants = self.grants.write().await;
        grants.retain(|_, grant| grant.expires_at > now);
        if let Some(grant) = grants.values_mut().find(|grant| {
            grant.preview_id == record.id
                && grant.generation == generation
                && grant.browser_only == browser_only
                && grant.workbench_origin == workbench_origin
                && grant.origin.scheme() == base.scheme()
                && grant.origin.port_or_known_default() == base.port_or_known_default()
                && grant.origin.host_str().is_some_and(|host| {
                    host.ends_with(&format!(".{}", base.host_str().unwrap_or_default()))
                })
        }) {
            grant.expires_at = expires_at;
            return Ok(PreviewAccessResponse {
                url: grant.origin.to_string(),
                expires_at,
                generation,
            });
        }
        // Issuance can race a stop/restart after its registry snapshot was
        // loaded. A late snapshot must never evict a newer run's grants.
        // Control mutations revoke grants explicitly; every proxy/agent use
        // also checks the live registry generation. Expired entries are
        // collected above, including any unusable raced issuance.
        if grants.len() >= MAX_GRANTS {
            return Err(AppError::too_many_requests(
                "too many active preview grants",
            ));
        }
        let host = format!(
            "p-{}.{}",
            uuid::Uuid::new_v4().simple(),
            base.host_str().unwrap_or_default()
        );
        let mut origin = base;
        origin
            .set_host(Some(&host))
            .map_err(|_| AppError::bad_request("invalid preview hostname"))?;
        if origin.origin() == backend.origin() || origin.origin() == workbench.origin() {
            return Err(AppError::forbidden(
                "preview origin must be isolated from the workbench and backend",
            ));
        }
        let grant = PreviewGrant {
            preview_id: record.id.clone(),
            generation: generation.clone(),
            origin: origin.clone(),
            workbench_origin,
            expires_at,
            browser_only,
        };
        grants.insert(host, grant);
        Ok(PreviewAccessResponse {
            url: origin.to_string(),
            expires_at,
            generation,
        })
    }

    pub(crate) async fn lookup(&self, host: &str) -> Option<PreviewGrant> {
        self.grants
            .read()
            .await
            .get(&host.trim_end_matches('.').to_ascii_lowercase())
            .filter(|grant| grant.expires_at > chrono::Utc::now().timestamp_millis())
            .cloned()
    }

    pub(crate) async fn revoke_host(&self, host: &str) {
        self.grants
            .write()
            .await
            .remove(&host.trim_end_matches('.').to_ascii_lowercase());
    }

    pub(crate) async fn hosts_for_preview(&self, preview_id: &str) -> Vec<String> {
        self.grants
            .read()
            .await
            .iter()
            .filter(|(_, grant)| grant.preview_id == preview_id)
            .map(|(host, _)| host.clone())
            .collect()
    }

    pub(crate) async fn revoke_hosts(&self, hosts: &[String]) {
        let mut grants = self.grants.write().await;
        for host in hosts {
            grants.remove(host);
        }
    }

    pub(crate) async fn revoke_hosts_except_generation(&self, hosts: &[String], generation: &str) {
        let mut grants = self.grants.write().await;
        for host in hosts {
            if grants
                .get(host)
                .is_some_and(|grant| grant.generation != generation)
            {
                grants.remove(host);
            }
        }
    }

    pub(crate) async fn revoke(&self, preview_id: &str) {
        self.grants
            .write()
            .await
            .retain(|_, grant| grant.preview_id != preview_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record() -> PreviewSessionRecord {
        serde_json::from_value(serde_json::json!({
            "id":"demo", "directory":"/work", "runDirectory":"/work",
            "agenaSessionId":"7", "state":"running",
            "proxyBasePath":"/api/v1/workbench/preview/s/demo/",
            "targetUrl":"http://127.0.0.1:5173/", "pid":123,
            "command":"node", "args":["server.js"], "logsPath":"/tmp/preview.log",
            "startedAt":10, "updatedAt":10
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn local_grant_is_isolated_reused_and_revocable() {
        let access = PreviewAccess::new(None, 8080).unwrap();
        let record = record();
        let first = access
            .issue(&record, "http://127.0.0.1:8080", "http://localhost:3000")
            .await
            .unwrap();
        let url = Url::parse(&first.url).unwrap();
        assert!(url.host_str().unwrap().starts_with("p-"));
        assert!(url.host_str().unwrap().ends_with(".localhost"));
        assert_eq!(url.port(), Some(8080));
        assert_eq!(url.path(), "/");
        assert!(url.query().is_none());
        assert_eq!(
            first.url,
            access
                .issue(&record, "http://127.0.0.1:8080", "http://localhost:3000")
                .await
                .unwrap()
                .url
        );
        assert!(access.lookup(url.host_str().unwrap()).await.is_some());
        access.revoke("demo").await;
        assert!(access.lookup(url.host_str().unwrap()).await.is_none());
        assert!(access.is_preview_host(url.host_str().unwrap()));
    }

    #[tokio::test]
    async fn racing_old_issuance_and_revocation_cannot_remove_the_new_run() {
        let access = PreviewAccess::new(None, 8080).unwrap();
        let mut record = record();
        let first = access
            .issue(&record, "http://localhost:8080", "http://localhost:8080")
            .await
            .unwrap();
        let old = record.clone();
        record.started_at = Some(11);
        let second = access
            .issue(&record, "http://localhost:8080", "http://localhost:8080")
            .await
            .unwrap();
        assert_ne!(first.generation, second.generation);
        assert_ne!(first.url, second.url);
        // An old request can finish issuing after the replacement run did.
        // Its grant is rejected by live generation checks, not by deleting
        // every differently-versioned grant from this registry snapshot.
        access
            .issue(&old, "http://localhost:8080", "http://localhost:8080")
            .await
            .unwrap();
        access
            .revoke_host(Url::parse(&first.url).unwrap().host_str().unwrap())
            .await;
        assert!(
            access
                .lookup(Url::parse(&first.url).unwrap().host_str().unwrap())
                .await
                .is_none()
        );
        assert!(
            access
                .lookup(Url::parse(&second.url).unwrap().host_str().unwrap())
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn mutation_revocation_does_not_remove_grants_issued_after_its_snapshot() {
        let access = PreviewAccess::new(None, 8080).unwrap();
        let mut record = record();
        let old = access
            .issue(&record, "http://localhost:8080", "http://localhost:8080")
            .await
            .unwrap();
        let hosts = access.hosts_for_preview(&record.id).await;
        // A no-op successful start must not invalidate the same running URL.
        access
            .revoke_hosts_except_generation(&hosts, &old.generation)
            .await;
        assert!(
            access
                .lookup(Url::parse(&old.url).unwrap().host_str().unwrap())
                .await
                .is_some()
        );
        record.started_at = Some(11);
        let new = access
            .issue(&record, "http://localhost:8080", "http://localhost:8080")
            .await
            .unwrap();
        access
            .revoke_hosts_except_generation(&hosts, &new.generation)
            .await;
        assert!(
            access
                .lookup(Url::parse(&old.url).unwrap().host_str().unwrap())
                .await
                .is_none()
        );
        assert!(
            access
                .lookup(Url::parse(&new.url).unwrap().host_str().unwrap())
                .await
                .is_some()
        );
        // PUT may deliberately revoke a captured same-generation host. It
        // still cannot revoke a later grant outside its captured host set.
        access.revoke_hosts(&hosts).await;
        assert!(
            access
                .lookup(Url::parse(&new.url).unwrap().host_str().unwrap())
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn browser_and_human_grants_never_share_an_origin() {
        let access = PreviewAccess::new(None, 8080).unwrap();
        let record = record();
        let human = access
            .issue(&record, "http://localhost:8080", "http://localhost:8080")
            .await
            .unwrap();
        let browser = access
            .issue_for_browser(&record, "http://localhost:8080")
            .await
            .unwrap();
        assert_ne!(human.url, browser.url);
        let grant = access
            .lookup(Url::parse(&browser.url).unwrap().host_str().unwrap())
            .await
            .unwrap();
        assert!(grant.browser_only);
        assert_eq!(
            browser.url,
            access
                .issue_for_browser(&record, "http://localhost:8080")
                .await
                .unwrap()
                .url
        );
    }

    #[tokio::test]
    async fn remote_access_requires_explicit_origin() {
        let record = record();
        assert!(
            PreviewAccess::new(None, 8080)
                .unwrap()
                .issue(
                    &record,
                    "https://agena.example.com",
                    "https://ui.example.com"
                )
                .await
                .is_err()
        );
        let access = PreviewAccess::new(Some("https://preview.example.com"), 8080).unwrap();
        let result = access
            .issue(
                &record,
                "https://agena.example.com",
                "https://ui.example.com",
            )
            .await
            .unwrap();
        assert!(
            Url::parse(&result.url)
                .unwrap()
                .host_str()
                .unwrap()
                .ends_with(".preview.example.com")
        );
        assert!(access.is_preview_host("expired.preview.example.com"));
    }

    #[test]
    fn origins_reject_credentials_paths_and_opaque_origins() {
        for raw in [
            "javascript:alert(1)",
            "https://user:password@example.com",
            "https://example.com/path",
            "https://example.com/?x=1",
            "null",
        ] {
            assert!(parse_origin(raw).is_err(), "{raw}");
        }
        assert!(PreviewAccess::new(Some("http://127.0.0.1:8081"), 8080).is_err());
    }

    #[tokio::test]
    async fn configured_dns_root_dot_uses_the_same_dispatch_and_lookup_host() {
        let access = PreviewAccess::new(Some("https://Preview.Example.Com."), 8080).unwrap();
        let grant = access
            .issue(
                &record(),
                "https://agena.example.com",
                "https://agena.example.com",
            )
            .await
            .unwrap();
        let host = Url::parse(&grant.url)
            .unwrap()
            .host_str()
            .unwrap()
            .to_owned();
        assert!(host.ends_with(".preview.example.com"));
        assert!(access.is_preview_host(&host));
        assert!(access.is_preview_host(&format!("{host}.")));
        assert!(access.lookup(&format!("{host}.")).await.is_some());
    }

    #[tokio::test]
    async fn real_control_listener_cannot_be_wrapped_as_a_preview() {
        let access = PreviewAccess::new(None, 3210).unwrap();
        for target in [
            "http://localhost:3210/",
            "http://127.0.0.1:3210/",
            "http://[::1]:3210/",
            "https://127.0.0.1:3210/",
        ] {
            let mut record = record();
            record.target_url = target.into();
            assert!(access.validate_record(&record).is_err(), "{target}");
            assert!(
                access
                    .issue(&record, "http://localhost:9999", "http://localhost:9999")
                    .await
                    .is_err()
            );
            assert!(
                access
                    .issue_for_browser(&record, "http://localhost:9999")
                    .await
                    .is_err()
            );
        }
        let default_http = PreviewAccess::new(None, 80).unwrap();
        assert!(
            default_http
                .validate_upstream(&Url::parse("http://localhost/").unwrap())
                .is_err()
        );
        let default_https = PreviewAccess::new(None, 443).unwrap();
        assert!(
            default_https
                .validate_upstream(&Url::parse("https://[::1]/").unwrap())
                .is_err()
        );
        assert!(
            access
                .validate_upstream(&Url::parse("http://127.0.0.1:5173/").unwrap())
                .is_ok()
        );
        assert!(
            access
                .validate_upstream(&Url::parse("http://user:secret@127.0.0.1:5173/").unwrap())
                .is_err()
        );
    }
}
