//! Narrow browser authorization for a host-owned preview run. A preview ID
//! never becomes a general exception to the public-network request policy.

use super::{BrowserOpenInput, ManagedPreviewTarget, PluginError, SdkResult};

pub(super) const POLICY_DESCRIPTION: &str = "Managed preview: document navigation stays on the exact preview origin; external fetch/WebSocket connections, frames, workers and popups are restricted. Access still requires live host network permission for the actual upstream URL. Other HTTP resources retain public-network permission checks. A policy-blocked request is not an application failure.";

pub(super) enum OpenTarget<'a> {
    Public(&'a str),
    Preview(&'a str),
}

pub(super) fn open_target(input: &BrowserOpenInput) -> SdkResult<OpenTarget<'_>> {
    match (&input.url, &input.preview_id) {
        (Some(url), None) if !url.trim().is_empty() => Ok(OpenTarget::Public(url.trim())),
        (None, Some(id)) if !id.trim().is_empty() => Ok(OpenTarget::Preview(id.trim())),
        _ => Err(PluginError::invalid_params(
            "supply exactly one nonempty url or preview_id; use browser_list to discover owned previews",
        )),
    }
}

pub(super) fn denied(message: impl Into<String>) -> PluginError {
    PluginError::from_kind(
        agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
        format!("managed preview access denied: {}", message.into()),
    )
}

/// An interrupted open must not leave a capability-bearing target running.
/// Closing is asynchronous, so cancellation delegates cleanup to the runtime.
pub(super) struct OpeningGuard {
    state: std::sync::Arc<super::BrowserActivityState>,
    target: Option<String>,
    host: std::sync::Arc<dyn super::HostClient>,
}

impl OpeningGuard {
    pub(super) fn new(
        state: std::sync::Arc<super::BrowserActivityState>,
        target: String,
        host: std::sync::Arc<dyn super::HostClient>,
    ) -> Self {
        Self {
            state,
            target: Some(target),
            host,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.target = None;
    }
}

impl Drop for OpeningGuard {
    fn drop(&mut self) {
        let Some(target) = self.target.take() else {
            return;
        };
        let state = self.state.clone();
        let host = self.host.clone();
        tokio::spawn(async move {
            match state
                .close_session(
                    &target,
                    "Incomplete browser open was cancelled.",
                    host.as_ref(),
                )
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    // A failed initialization may already have closed the
                    // Chrome target before this final cleanup runs.
                    state.clients.lock().await.remove(&target);
                    state.previews.lock().await.remove(&target);
                    state.actions.lock().await.remove(&target);
                    state.connecting.lock().await.remove(&target);
                    state.meta.lock().await.remove(&target);
                    state.logs.lock().await.remove(&target);
                }
                Err(error) => {
                    super::log_secondary_plugin_failure("close cancelled browser open", &error)
                }
            }
        });
    }
}

/// Scope matching alone is not permission to contact the upstream server.
async fn within_scope(
    preview: Option<&ManagedPreviewTarget>,
    url: &url::Url,
    is_document: bool,
) -> SdkResult<bool> {
    let Some(preview) = preview else {
        return Ok(false);
    };
    preview.validate().await.map_err(denied)?;
    if preview.contains_url(url) {
        return Ok(true);
    }
    if is_document {
        return Err(denied(
            "document navigation is outside the authorized preview origin; use a separate browser_open with a public URL",
        ));
    }
    Ok(false)
}

/// Owning a running child does not establish ownership of an arbitrary port.
/// Always ask the host about the real upstream, not the capability hostname;
/// the host validates the active call authority and its current permission rule.
pub(super) async fn require_upstream_permission(
    preview: &ManagedPreviewTarget,
    host: &dyn super::HostClient,
) -> SdkResult<()> {
    preview.validate().await.map_err(denied)?;
    let context = agena_plugin_host::sdk::host_api::current_host_callback_context();
    if context.is_none_or(|context| context.session_id.is_none() || context.call_id.is_none()) {
        return Err(denied(
            "upstream network authorization requires an active tool call",
        ));
    }
    host.require_network_permission(preview.upstream_url().to_string())
        .await?;
    // Permission approval can wait; stopping or replacing the run during that
    // wait must not activate an already retired grant.
    preview.validate().await.map_err(denied)
}

/// True skips only the public-IP prohibition. Both the run lease and ordinary
/// user/host permission must have passed before the request can continue.
pub(super) async fn authorize_request(
    preview: Option<&ManagedPreviewTarget>,
    url: &url::Url,
    is_document: bool,
    host: &dyn super::HostClient,
) -> SdkResult<bool> {
    if !within_scope(preview, url, is_document).await? {
        return Ok(false);
    }
    let preview = preview.expect("scope matching requires a preview");
    require_upstream_permission(preview, host).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_plugin_host::sdk::{
        ToolInvokeOutput, async_trait,
        hooks::{EventEnvelope, EventFilter},
        host_api::{
            EventSubscription, HostCallbackContext, HostClient, LogLevel,
            run_in_isolated_host_callback_context,
        },
    };
    use agena_web::managed_preview::ManagedPreviewLease;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    struct PermissionHost {
        allowed: AtomicBool,
        requests: std::sync::Mutex<Vec<String>>,
    }

    impl PermissionHost {
        fn allowed() -> Self {
            Self {
                allowed: AtomicBool::new(true),
                requests: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl HostClient for PermissionHost {
        async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}
        async fn publish_event(&self, _event: EventEnvelope) -> SdkResult<()> {
            Err(denied("unused host capability"))
        }
        async fn subscribe_events(&self, _filter: EventFilter) -> SdkResult<EventSubscription> {
            Err(denied("unused host capability"))
        }
        async fn read_config(&self, _path: Option<String>) -> SdkResult<serde_json::Value> {
            Err(denied("unused host capability"))
        }
        async fn invoke_tool(
            &self,
            _tool: String,
            _input: serde_json::Value,
        ) -> SdkResult<ToolInvokeOutput> {
            Err(denied("unused host capability"))
        }
        async fn require_network_permission(&self, target: String) -> SdkResult<()> {
            self.requests.lock().unwrap().push(target);
            if self.allowed.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err(denied("upstream denied by user network policy"))
            }
        }
    }

    struct Lease(AtomicBool);
    impl ManagedPreviewLease for Lease {
        fn validate(&self) -> futures_util::future::BoxFuture<'_, Result<(), String>> {
            Box::pin(async move {
                if self.0.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err("run revoked".into())
                }
            })
        }
    }

    fn target() -> (ManagedPreviewTarget, Arc<Lease>) {
        let lease = Arc::new(Lease(AtomicBool::new(true)));
        (
            ManagedPreviewTarget::new(
                "preview-1".into(),
                "generation-1".into(),
                url::Url::parse("http://p-owned.localhost:8080/").unwrap(),
                lease.clone(),
                url::Url::parse("http://127.0.0.1:5173/").unwrap(),
            ),
            lease,
        )
    }

    #[test]
    fn browser_open_requires_one_typed_target() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"url":"", "preview_id":null}),
            serde_json::json!({"preview_id":"  "}),
            serde_json::json!({"url":"https://example.com", "preview_id":"preview-1"}),
        ] {
            let input: BrowserOpenInput = serde_json::from_value(value).unwrap();
            assert!(open_target(&input).is_err());
        }
        let public: BrowserOpenInput =
            serde_json::from_value(serde_json::json!({"url":"https://example.com"})).unwrap();
        assert!(matches!(
            open_target(&public).unwrap(),
            OpenTarget::Public("https://example.com")
        ));
        let preview: BrowserOpenInput =
            serde_json::from_value(serde_json::json!({"preview_id":" preview-1 "})).unwrap();
        assert!(matches!(
            open_target(&preview).unwrap(),
            OpenTarget::Preview("preview-1")
        ));
    }

    #[tokio::test]
    async fn managed_requests_check_exact_origin_and_each_run_lease() {
        let (preview, lease) = target();
        let own = url::Url::parse("http://p-owned.localhost:8080/api/data").unwrap();
        assert!(within_scope(Some(&preview), &own, true).await.unwrap());
        assert!(within_scope(Some(&preview), &own, false).await.unwrap());
        for raw in [
            "http://localhost:8080/api/v1/sessions",
            "http://p-other.localhost:8080/",
            "http://p-owned.localhost:9999/",
            "http://127.0.0.1:3000/",
            "http://user@p-owned.localhost:8080/",
            "https://example.com/",
            "file:///etc/passwd",
        ] {
            let url = url::Url::parse(raw).unwrap();
            let error = within_scope(Some(&preview), &url, true).await.unwrap_err();
            assert_eq!(
                error.kind,
                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied
            );
            assert!(!within_scope(Some(&preview), &url, false).await.unwrap());
        }
        lease.0.store(false, Ordering::Release);
        assert!(within_scope(Some(&preview), &own, true).await.is_err());
        assert!(within_scope(Some(&preview), &own, false).await.is_err());
        // Revocation also stops new third-party resources from an old page.
        assert!(
            within_scope(
                Some(&preview),
                &url::Url::parse("https://example.com/x.js").unwrap(),
                false
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn no_preview_grant_never_authorizes_a_private_request() {
        let url = url::Url::parse("http://127.0.0.1:3000/").unwrap();
        assert!(!within_scope(None, &url, true).await.unwrap());
        let error = super::super::validate_public_network_target(
            &url,
            &super::super::network_policy::WebNetworkConfig::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.kind,
            agena_plugin_host::sdk::PluginErrorKind::PolicyDenied
        );
    }

    #[tokio::test]
    async fn revoked_preview_blocks_non_network_actions_before_connecting_cdp() {
        let (preview, lease) = target();
        let plugin = super::super::WebPlugin::new();
        plugin
            .browser_state
            .previews
            .lock()
            .await
            .insert("owned-page".into(), preview);
        lease.0.store(false, Ordering::Release);
        let error = plugin
            .validate_browser_preview_state("owned-page")
            .await
            .unwrap_err();
        assert_eq!(
            error.kind,
            agena_plugin_host::sdk::PluginErrorKind::PolicyDenied
        );
        // A different target receives no private-network exception from the
        // existence of another page's lease.
        assert!(plugin.browser_preview("another-page").await.is_none());
    }

    #[tokio::test]
    async fn managed_requests_authorize_the_real_upstream_on_every_request() {
        let (preview, _lease) = target();
        let host = PermissionHost::allowed();
        let context = HostCallbackContext {
            session_id: Some(31),
            call_id: Some(47),
            ..Default::default()
        };
        run_in_isolated_host_callback_context(context, async {
            assert!(
                authorize_request(Some(&preview), preview.url(), true, &host)
                    .await
                    .unwrap()
            );
            host.allowed.store(false, Ordering::Release);
            let error = authorize_request(Some(&preview), preview.url(), false, &host)
                .await
                .unwrap_err();
            assert_eq!(
                error.kind,
                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied
            );
        })
        .await;
        assert_eq!(
            *host.requests.lock().unwrap(),
            vec![
                "http://127.0.0.1:5173/".to_string(),
                "http://127.0.0.1:5173/".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn managed_background_requests_need_an_active_host_call() {
        let (preview, _lease) = target();
        let host = PermissionHost::allowed();
        assert!(
            authorize_request(Some(&preview), preview.url(), false, &host)
                .await
                .is_err()
        );
        run_in_isolated_host_callback_context(HostCallbackContext::default(), async {
            assert!(
                authorize_request(Some(&preview), preview.url(), false, &host)
                    .await
                    .is_err()
            );
        })
        .await;
        assert!(host.requests.lock().unwrap().is_empty());
    }
}
