use super::*;
use std::{future::Future, pin::Pin, sync::Mutex};

struct HeaderPolicyProbe {
    tag: &'static str,
    observed: Arc<Mutex<Vec<BTreeMap<String, String>>>>,
    inject_credential: bool,
}

impl ProviderRequestHeaderHook for HeaderPolicyProbe {
    fn resolve(
        &self,
        _provider_id: &str,
        headers: BTreeMap<String, String>,
        _cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = Result<BTreeMap<String, String>, ProviderError>> + Send + '_>>
    {
        Box::pin(async move {
            self.observed.lock().unwrap().push(headers.clone());
            let mut updated = headers;
            updated.insert("x-hook-owner".into(), self.tag.into());
            if self.inject_credential {
                updated.insert("X-API-KEY".into(), "forged-key".into());
            }
            Ok(updated)
        })
    }
}

#[tokio::test]
async fn header_hooks_cannot_read_or_mutate_provider_credentials() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let hook = Arc::new(HeaderPolicyProbe {
        tag: "session-a",
        observed: observed.clone(),
        inject_credential: false,
    });
    let input = BTreeMap::from([
        ("Authorization".into(), "Bearer provider-secret".into()),
        ("x-api-key".into(), "provider-key".into()),
        ("x-client-name".into(), "agena".into()),
    ]);
    let output = with_request_header_hook(hook, async {
        resolved_request_headers_async("provider", input)
            .await
            .unwrap()
    })
    .await;
    assert_eq!(output["Authorization"], "Bearer provider-secret");
    assert_eq!(output["x-api-key"], "provider-key");
    assert_eq!(output["x-client-name"], "agena");
    assert_eq!(output["x-hook-owner"], "session-a");

    {
        // End the synchronous mutex guard before the next asynchronous hook
        // invocation. Holding it across await can block another resolver.
        let seen = observed.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].len(), 1);
        assert_eq!(seen[0]["x-client-name"], "agena");
    }

    let rejected = with_request_header_hook(
        Arc::new(HeaderPolicyProbe {
            tag: "session-a",
            observed: Arc::new(Mutex::new(Vec::new())),
            inject_credential: true,
        }),
        async { resolved_request_headers_async("provider", BTreeMap::new()).await },
    )
    .await;
    assert!(matches!(rejected, Err(ProviderError::Config(_))));
}

#[tokio::test]
async fn concurrent_sessions_have_isolated_header_policies() {
    let policy = |tag: &'static str| {
        Arc::new(HeaderPolicyProbe {
            tag,
            observed: Arc::new(Mutex::new(Vec::new())),
            inject_credential: false,
        }) as Arc<dyn ProviderRequestHeaderHook>
    };
    let left = with_request_header_hook(policy("left"), async {
        tokio::task::yield_now().await;
        resolved_request_headers_async("provider", BTreeMap::new())
            .await
            .unwrap()
    });
    let right = with_request_header_hook(policy("right"), async {
        tokio::task::yield_now().await;
        resolved_request_headers_async("provider", BTreeMap::new())
            .await
            .unwrap()
    });
    let (left, right) = tokio::join!(left, right);
    assert_eq!(left["x-hook-owner"], "left");
    assert_eq!(right["x-hook-owner"], "right");
}
