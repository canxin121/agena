use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture(status: agena_web::PageContentStatus, rendered: bool) -> FetchedPage {
    let url = url::Url::parse("https://example.test/").unwrap();
    let mut page = agena_web::extract_page_from_body(
        &url,
        &url,
        "text/plain",
        200,
        false,
        rendered,
        "中文 🦀 readable fixture",
        None,
        None,
    );
    page.content_status = status;
    page
}

#[tokio::test]
async fn rendering_is_conditional_and_explicit_modes_are_respected() {
    use agena_web::PageContentStatus::{Blocked, Empty, Readable, RequiresJavascript};
    for (explicit, enabled, status, expected) in [
        (None, true, RequiresJavascript, vec![false, true]),
        (None, false, RequiresJavascript, vec![false]),
        (Some(false), true, RequiresJavascript, vec![false]),
        (Some(true), false, Readable, vec![true]),
        (None, true, Readable, vec![false]),
        (None, true, Blocked, vec![false]),
        (None, true, Empty, vec![false]),
    ] {
        let calls = std::sync::Mutex::new(Vec::new());
        let result = select_fetch(explicit, enabled, Duration::from_secs(1), |render| {
            calls.lock().unwrap().push(render);
            async move { Ok(fixture(if render { Readable } else { status }, render)) }
        })
        .await
        .unwrap();
        assert_eq!(*calls.lock().unwrap(), expected);
        assert_eq!(result.rendered, *expected.last().unwrap());
    }
    let calls = AtomicUsize::new(0);
    let _ = select_fetch(None, true, Duration::from_secs(1), |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        async {
            let mut page = fixture(RequiresJavascript, false);
            page.status = 403;
            Ok(page)
        }
    })
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn browser_retry_failure_retains_http_status_and_shared_deadline() {
    let calls = AtomicUsize::new(0);
    let result = select_fetch(None, true, Duration::from_millis(80), |render| {
        calls.fetch_add(1, Ordering::SeqCst);
        async move {
            tokio::time::sleep(Duration::from_millis(if render { 80 } else { 30 })).await;
            Ok(fixture(
                agena_web::PageContentStatus::RequiresJavascript,
                render,
            ))
        }
    })
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(!result.rendered);
    assert!(
        result
            .warnings
            .iter()
            .any(|s| s.contains("shared fetch time budget"))
    );
}

struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn batch_bounds_parallelism_preserves_order_and_cancels_in_flight_reads() {
    use agena_plugin_host::sdk::host_api::{
        HostCallbackContext, current_host_callback_context, run_in_isolated_host_callback_context,
    };
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let urls = (0..8).map(|i| i.to_string()).collect();
    let results = run_in_isolated_host_callback_context(
        HostCallbackContext {
            session_id: Some(55),
            ..Default::default()
        },
        batch(urls, 3, |raw| {
            let active = &active;
            let peak = &peak;
            async move {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                let _guard = Active(active);
                peak.fetch_max(now, Ordering::SeqCst);
                assert_eq!(
                    current_host_callback_context().unwrap().session_id,
                    Some(55)
                );
                tokio::time::sleep(Duration::from_millis(if raw == "0" { 15 } else { 2 })).await;
                if raw == "1" { Err(raw) } else { Ok(raw) }
            }
        }),
    )
    .await;
    assert_eq!(peak.load(Ordering::SeqCst), 3);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(results[0], Ok("0".into()));
    assert_eq!(results[1], Err("1".into()));
    assert_eq!(results[7], Ok("7".into()));
    let cancel = batch(vec!["x".into(); 8], 3, |_| async {
        active.fetch_add(1, Ordering::SeqCst);
        let _guard = Active(&active);
        std::future::pending::<()>().await;
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(10), cancel)
            .await
            .is_err()
    );
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[test]
fn batch_deduplicates_before_dispatch_and_rejects_excess_work() {
    assert_eq!(
        unique_urls(&[
            "https://example.test#first".into(),
            " https://example.test/ ".into(),
            "invalid".into()
        ])
        .unwrap(),
        vec!["https://example.test/", "invalid"]
    );
    assert!(unique_urls(&[]).is_err());
    assert!(unique_urls(&vec!["https://example.test/".into(); 9]).is_err());
    assert!(bounded(Some(0), 4, 8, "concurrency").is_err());
    assert!(bounded(Some(9), 4, 8, "concurrency").is_err());
}

struct PolicyHost;
#[async_trait]
impl HostClient for PolicyHost {
    async fn log(
        &self,
        _: agena_plugin_host::sdk::host_api::LogLevel,
        _: String,
        _: serde_json::Value,
    ) {
    }
    async fn publish_event(&self, _: agena_plugin_host::sdk::EventEnvelope) -> SdkResult<()> {
        Ok(())
    }
    async fn subscribe_events(
        &self,
        _: agena_plugin_host::sdk::EventFilter,
    ) -> SdkResult<agena_plugin_host::sdk::host_api::EventSubscription> {
        Err(PluginError::internal("unused"))
    }
    async fn read_config(&self, _: Option<String>) -> SdkResult<serde_json::Value> {
        Err(PluginError::internal("unused"))
    }
    async fn invoke_tool(&self, _: String, _: serde_json::Value) -> SdkResult<ToolInvokeOutput> {
        Err(PluginError::internal("unused"))
    }
    async fn ask_user(
        &self,
        _: agena_plugin_host::sdk::host_api::AskUserRequest,
    ) -> SdkResult<agena_plugin_host::sdk::host_api::AskUserResponse> {
        Err(PluginError::internal("unused"))
    }
    async fn check_network_permission(
        &self,
        raw: String,
    ) -> SdkResult<agena_plugin_host::sdk::host_api::PermissionQuery> {
        use agena_plugin_host::sdk::host_api::PermissionQuery;
        Ok(if raw.contains("denied.test") {
            PermissionQuery::Deny {
                reason: "fixture denied".into(),
            }
        } else {
            PermissionQuery::Allow {}
        })
    }
}

#[tokio::test]
async fn continuations_match_text_payload_and_reauthorize_redirect_destination() {
    let plugin = WebPlugin::new();
    assert!(
        plugin
            .state
            .set(WebPluginState::new(
                WebConfig::default(),
                Arc::new(PolicyHost)
            ))
            .is_ok()
    );
    let page = fixture(agena_web::PageContentStatus::Readable, false);
    let original = page.markdown.clone();
    let first = plugin.page_output(page.clone(), None, 3).await.unwrap();
    let payload = first.payload.unwrap();
    assert!(
        first
            .output_text
            .contains(payload["markdown"].as_str().unwrap())
    );
    let id = payload["page_id"].as_str().unwrap();
    let second = plugin
        .invoke_read(&ReadPageInput {
            page_id: id.into(),
            offset: 3,
            max_chars: Some(24000),
        })
        .await
        .unwrap();
    assert_eq!(
        format!(
            "{}{}",
            payload["markdown"].as_str().unwrap(),
            second.payload.unwrap()["markdown"].as_str().unwrap()
        ),
        original
    );
    let mut denied = page;
    denied.final_url = "https://denied.test/private".into();
    let id = plugin
        .state()
        .unwrap()
        .snapshots
        .insert(denied)
        .await
        .unwrap();
    assert!(
        plugin
            .invoke_read(&ReadPageInput {
                page_id: id,
                offset: 0,
                max_chars: None
            })
            .await
            .is_err()
    );
}

#[test]
fn live_schemas_advertise_bounds_and_continuation() {
    use agena_plugin_host::sdk::Plugin;
    let manifest = WebPlugin::new().manifest();
    for (name, field, max) in [
        ("fetch", "max_chars", 24000),
        ("fetch_many", "concurrency", 8),
        ("read", "max_chars", 24000),
        ("query", "max_results", 20),
        ("crawl", "concurrency", 8),
    ] {
        let tool = manifest
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap();
        assert_eq!(tool.input_schema()["properties"][field]["maximum"], max);
        assert!(
            tool.help_text().unwrap().contains(if name == "read" {
                "next_offset"
            } else {
                "web.read"
            }),
            "{name} help must describe continuation"
        );
    }
}

#[tokio::test]
async fn local_query_returns_readable_evidence_and_omits_denied_documents() {
    let workspace = tempfile::tempdir().unwrap();
    let plugin = WebPlugin::new();
    assert!(
        plugin
            .workspace_root
            .set(workspace.path().to_path_buf())
            .is_ok()
    );
    assert!(
        plugin
            .state
            .set(WebPluginState::new(
                WebConfig::default(),
                Arc::new(PolicyHost)
            ))
            .is_ok()
    );
    let store = plugin.store().unwrap();
    // Only this randomly named fixture's crawl directory is removed.
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(store.dir().to_path_buf());
    for host in ["example.test", "denied.test"] {
        let url = url::Url::parse(&format!("https://{host}/")).unwrap();
        let body = format!(
            "{}\n\n## Evidence\n\nneedle evidence with 中文 context on {host}",
            "Introduction paragraph. ".repeat(60)
        );
        let page = agena_web::extract_page_from_body(
            &url,
            &url,
            "text/plain",
            200,
            false,
            false,
            &body,
            None,
            None,
        );
        store
            .save_document(&agena_web::StoredDocument::from_fetched_page(page, 0, 400))
            .unwrap();
    }
    store.rebuild_index().unwrap();
    let output = plugin
        .invoke_query(&QueryCrawlInput {
            query: "needle".into(),
            max_results: Some(20),
        })
        .await
        .unwrap();
    let payload = output.payload.unwrap();
    assert_eq!(payload["omitted_count"], 1);
    assert_eq!(payload["results"].as_array().unwrap().len(), 1);
    assert!(!output.output_text.contains("denied.test"));
    let result = &payload["results"][0];
    let offset = result["read_offset"].as_u64().unwrap() as usize;
    assert!(offset > 0);
    let read = plugin
        .invoke_read(&ReadPageInput {
            page_id: result["page_id"].as_str().unwrap().into(),
            offset,
            max_chars: None,
        })
        .await
        .unwrap();
    assert!(
        read.payload.unwrap()["markdown"]
            .as_str()
            .unwrap()
            .contains("needle evidence")
    );
}
