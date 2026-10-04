use super::*;

#[test]
fn untrusted_bridge_diagnostics_never_escape() {
    for bytes in [
        br#"{"ok":false,"error":"PASSWORD_UNDER_TEST"}"#.as_slice(),
        br#"{"ok":true,"secret":"PASSWORD_UNDER_TEST"}"#,
        b"Traceback: PASSWORD_UNDER_TEST",
    ] {
        let error = decode(bytes).unwrap_err();
        assert!(!error.diagnostic_message().contains("PASSWORD_UNDER_TEST"));
    }
    assert_eq!(decode(br#"{"ok":true}"#).unwrap()["backend"], "playwright");
}

#[tokio::test]
#[ignore = "requires explicit AGENA_BROWSER_PYTHON with Playwright and a local Chrome"]
async fn real_playwright_preserves_owned_context_and_native_interception() {
    let python = std::env::var_os("AGENA_BROWSER_PYTHON").expect("explicit fixture interpreter");
    let endpoint =
        tokio::task::spawn_blocking(|| local_browser_endpoint(&LocalBrowserOptions::default()))
            .await
            .unwrap()
            .expect("Chrome must run, not silently skip");
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = shutdown_local_browser();
        }
    }
    let _cleanup = Cleanup;
    let root = CdpClient::connect(&endpoint, None, None).await.unwrap();
    let context = root
        .command(
            "Target.createBrowserContext",
            serde_json::json!({"disposeOnDetach":true}),
        )
        .await
        .unwrap();
    let context_id = context["browserContextId"].as_str().unwrap();
    let target = root
        .command(
            "Target.createTarget",
            serde_json::json!({"url":"about:blank","browserContextId":context_id}),
        )
        .await
        .unwrap();
    let target_id = target["targetId"].as_str().unwrap();
    let native = CdpClient::connect(&endpoint, Some(target_id), None)
        .await
        .unwrap();
    native.enable_navigation_interception().await.unwrap();
    native.evaluate(r#"(() => {
        document.body.innerHTML = '<input id="input" type="password"><button id="button">Click</button><div id="cover" style="position:fixed;inset:0;z-index:10;background:white"></div>';
        window.clicks = 0;
        document.querySelector('#button').onclick = () => window.clicks++;
        return true;
    })()"#).await.unwrap();
    let mut request = Request {
        endpoint: &endpoint,
        target_id,
        context_id,
        action: "click",
        selector: Some("#button"),
        frame_selector: None,
        element_expression: None,
        text: None,
        press_enter: false,
        timeout_ms: 1_500,
    };
    let error = run_with_python(&request, python.clone()).await.unwrap_err();
    assert!(error.diagnostic_message().contains("timed out"));
    assert_eq!(native.evaluate("window.clicks").await.unwrap(), 0);
    native
        .evaluate("document.querySelector('#cover').remove()")
        .await
        .unwrap();
    request.timeout_ms = 10_000;
    run_with_python(&request, python.clone()).await.unwrap();
    assert_eq!(native.evaluate("window.clicks").await.unwrap(), 1);

    request.action = "fill";
    request.selector = Some("#input");
    request.text = Some("秘密😀 PASSWORD_UNDER_TEST");
    let result = run_with_python(&request, python.clone()).await.unwrap();
    assert!(!result.to_string().contains("PASSWORD_UNDER_TEST"));
    assert_eq!(
        native
            .evaluate("document.querySelector('#input').value")
            .await
            .unwrap(),
        request.text.unwrap()
    );
    let snapshot = native
        .evaluate(&browser_snapshot_expression())
        .await
        .unwrap();
    assert!(!snapshot.to_string().contains("PASSWORD_UNDER_TEST"));
    let id = snapshot["snapshot_id"].as_str().unwrap();
    let index = snapshot["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == "button")
        .unwrap()["ref"]
        .as_u64()
        .unwrap() as u16;
    let expression = browser_element_expression(None, Some(index), Some(id)).unwrap();
    request.action = "click";
    request.selector = None;
    request.text = None;
    request.element_expression = Some(&expression);
    run_with_python(&request, python.clone()).await.unwrap();
    native
        .evaluate(&browser_snapshot_expression())
        .await
        .unwrap();
    let error = run_with_python(&request, python.clone()).await.unwrap_err();
    assert!(error.diagnostic_message().contains("stale"));

    // Readiness starts after bridge startup and checks visibility, not presence.
    native.evaluate("(() => { const x=document.createElement('div'); x.id='late'; x.style.display='none'; document.body.append(x); setTimeout(() => x.style.display='block', 2500); x.textContent='ready'; return true; })()").await.unwrap();
    request.action = "wait";
    request.selector = Some("#late");
    request.element_expression = None;
    run_with_python(&request, python.clone()).await.unwrap();
    assert_eq!(
        native
            .evaluate("getComputedStyle(document.querySelector('#late')).display")
            .await
            .unwrap(),
        "block"
    );
    native.evaluate(r#"(() => { const frame=document.createElement('iframe'); frame.id='frame'; frame.srcdoc='<input id="framed"><button>one</button><button>two</button>'; document.body.append(frame); return true; })()"#).await.unwrap();
    request.frame_selector = Some("#frame");
    request.action = "fill";
    request.selector = Some("#framed");
    request.text = Some("iframe 输入😀");
    run_with_python(&request, python.clone()).await.unwrap();
    assert_eq!(
        native
            .evaluate(
                "document.querySelector('#frame').contentDocument.querySelector('#framed').value"
            )
            .await
            .unwrap(),
        "iframe 输入😀"
    );
    request.action = "click";
    request.selector = Some("button");
    request.text = None;
    assert!(
        run_with_python(&request, python.clone())
            .await
            .unwrap_err()
            .diagnostic_message()
            .contains("multiple elements")
    );
    request.frame_selector = None;
    request.action = "wait";
    request.selector = Some("#late");
    request.context_id = "foreign-context";
    let error = run_with_python(&request, python.clone()).await.unwrap_err();
    assert!(error.diagnostic_message().contains("owned page"));
    request.context_id = context_id;
    let contexts = root
        .command("Target.getBrowserContexts", serde_json::json!({}))
        .await
        .unwrap();
    assert!(
        contexts["browserContextIds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == context_id)
    );

    native.evaluate("(() => { const a=document.createElement('a'); a.id='denied'; a.href='http://127.0.0.1:9/private'; a.textContent='Navigate'; document.body.append(a); return true; })()").await.unwrap();
    request.action = "click";
    request.selector = Some("#denied");
    // A failed navigation may make Playwright itself fail or complete its click.
    let _ = run_with_python(&request, python).await;
    assert!(
        !native.navigation_errors.lock().unwrap().is_empty(),
        "native interception must remain attached"
    );
    root.command(
        "Target.disposeBrowserContext",
        serde_json::json!({"browserContextId":context_id}),
    )
    .await
    .unwrap();
    eprintln!(
        "AUDIT_REAL_PLAYWRIGHT: covered clicks, Unicode fill/redaction, snapshot refs, delayed visibility, iframe input/strictness, context retention/scope, native navigation interception passed"
    );
}
