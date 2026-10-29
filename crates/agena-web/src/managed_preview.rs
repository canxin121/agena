//! Host-issued access to registered previews without relaxing public-web SSRF
//! policy. Providers live only as long as their owning application server.

use futures_util::future::BoxFuture;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
};
use url::Url;

/// A revocable lease supplied by the application that owns a preview run.
pub trait ManagedPreviewLease: Send + Sync {
    /// Recheck ownership, expiry and generation for every browser request.
    fn validate(&self) -> BoxFuture<'_, Result<(), String>>;
}

/// A preview target can only originate from a registered host provider.
#[derive(Clone)]
pub struct ManagedPreviewTarget {
    preview_id: String,
    generation: String,
    url: Url,
    lease: Arc<dyn ManagedPreviewLease>,
    upstream_url: Url,
}

impl std::fmt::Debug for ManagedPreviewTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Capability hostnames are intentionally excluded from diagnostics.
        f.debug_struct("ManagedPreviewTarget")
            .field("preview_id", &self.preview_id)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl ManagedPreviewTarget {
    /// Construct a target inside a trusted preview provider.
    pub fn new(
        preview_id: String,
        generation: String,
        url: Url,
        lease: Arc<dyn ManagedPreviewLease>,
        upstream_url: Url,
    ) -> Self {
        Self {
            preview_id,
            generation,
            url,
            lease,
            upstream_url,
        }
    }
    /// Registry identifier, scoped to the current workspace and owner.
    pub fn preview_id(&self) -> &str {
        &self.preview_id
    }
    /// Identity of the run authorized by this lease.
    pub fn generation(&self) -> &str {
        &self.generation
    }
    /// Isolated browser entry point; never a workbench administrative token.
    pub fn url(&self) -> &Url {
        &self.url
    }
    /// Real upstream to authorize through the caller's existing network policy.
    /// A live process handle does not prove ownership of a listening port.
    pub fn upstream_url(&self) -> &Url {
        &self.upstream_url
    }
    /// Match exactly this origin, with WS/WSS corresponding to HTTP/HTTPS.
    pub fn contains_url(&self, url: &Url) -> bool {
        let mut normalized = url.clone();
        let scheme = match url.scheme() {
            "ws" => "http",
            "wss" => "https",
            "http" => "http",
            "https" => "https",
            _ => return false,
        };
        if normalized.set_scheme(scheme).is_err()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return false;
        }
        normalized.origin() == self.url.origin()
    }
    /// Validate both exact network scope and the provider's live lease.
    pub async fn validate_url(&self, url: &Url) -> Result<(), String> {
        if !self.contains_url(url) {
            return Err("URL is outside this managed preview origin".into());
        }
        self.lease.validate().await
    }
    /// Validate the lease even for actions that do not trigger a navigation.
    pub async fn validate(&self) -> Result<(), String> {
        self.lease.validate().await
    }
}

/// Non-secret preview discovery for a workspace/session owner.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ManagedPreviewInfo {
    /// Preview registry identifier.
    pub preview_id: String,
    /// Current process state; this is not a browser verification result.
    pub state: String,
}

/// Implemented by the application server, not by untrusted page content.
pub trait ManagedPreviewProvider: Send + Sync {
    /// Return None for previews outside this workspace/session ownership scope.
    fn resolve(
        &self,
        workspace: PathBuf,
        session_id: String,
        preview_id: String,
    ) -> BoxFuture<'_, Result<Option<ManagedPreviewTarget>, String>>;
    /// List only previews belonging to the caller's workspace/session.
    fn list(
        &self,
        workspace: PathBuf,
        session_id: String,
    ) -> BoxFuture<'_, Result<Vec<ManagedPreviewInfo>, String>>;
}

type Providers = BTreeMap<u64, Weak<dyn ManagedPreviewProvider>>;
fn providers() -> &'static Mutex<Providers> {
    static PROVIDERS: OnceLock<Mutex<Providers>> = OnceLock::new();
    PROVIDERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Owning this guard keeps a provider available. Drop revokes discovery.
pub struct ManagedPreviewRegistration {
    id: u64,
    _provider: Arc<dyn ManagedPreviewProvider>,
}

/// Register an application-owned provider for the lifetime of its server.
pub fn register_managed_preview_provider(
    provider: Arc<dyn ManagedPreviewProvider>,
) -> ManagedPreviewRegistration {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    providers()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(id, Arc::downgrade(&provider));
    ManagedPreviewRegistration {
        id,
        _provider: provider,
    }
}

impl Drop for ManagedPreviewRegistration {
    fn drop(&mut self) {
        providers()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
    }
}

fn live_providers() -> Vec<Arc<dyn ManagedPreviewProvider>> {
    let mut map = providers()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    map.retain(|_, provider| provider.strong_count() > 0);
    map.values().filter_map(Weak::upgrade).collect()
}

/// Resolve a typed preview target without accepting arbitrary local URLs.
pub async fn resolve_managed_preview(
    workspace: &Path,
    session_id: &str,
    preview_id: &str,
) -> Result<ManagedPreviewTarget, String> {
    let mut found = None;
    for provider in live_providers() {
        if let Some(target) = provider
            .resolve(
                workspace.to_path_buf(),
                session_id.into(),
                preview_id.into(),
            )
            .await?
        {
            if found.is_some() {
                return Err("managed preview is ambiguous across running servers".into());
            }
            found = Some(target);
        }
    }
    found.ok_or_else(|| {
        "preview not found, not running, or not owned by this workspace/session".into()
    })
}

/// Discover available previews without issuing access capabilities.
pub async fn list_managed_previews(
    workspace: &Path,
    session_id: &str,
) -> Result<Vec<ManagedPreviewInfo>, String> {
    let mut result = Vec::new();
    for provider in live_providers() {
        result.extend(
            provider
                .list(workspace.to_path_buf(), session_id.into())
                .await?,
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    struct Lease(AtomicBool);
    impl ManagedPreviewLease for Lease {
        fn validate(&self) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async move {
                if self.0.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err("revoked".into())
                }
            })
        }
    }
    #[tokio::test]
    async fn exact_origin_and_revocation_are_both_enforced() {
        let lease = Arc::new(Lease(AtomicBool::new(true)));
        let target = ManagedPreviewTarget::new(
            "p".into(),
            "g".into(),
            Url::parse("http://p-secret.localhost:8080/").unwrap(),
            lease.clone(),
            Url::parse("http://127.0.0.1:5173/").unwrap(),
        );
        assert_eq!(target.upstream_url().as_str(), "http://127.0.0.1:5173/");
        assert!(
            target
                .validate_url(&Url::parse("http://p-secret.localhost:8080/api/items").unwrap())
                .await
                .is_ok()
        );
        assert!(target.contains_url(&Url::parse("ws://p-secret.localhost:8080/socket").unwrap()));
        for url in [
            "http://127.0.0.1:8080/",
            "http://p-other.localhost:8080/",
            "http://p-secret.localhost:9000/",
            "http://p-secret.localhost:8080.evil.example/",
            "http://user@p-secret.localhost:8080/",
        ] {
            if let Ok(url) = Url::parse(url) {
                assert!(target.validate_url(&url).await.is_err());
            }
        }
        lease.0.store(false, Ordering::Release);
        assert!(target.validate_url(target.url()).await.is_err());
    }
}
