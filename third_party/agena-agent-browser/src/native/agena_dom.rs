//! DOM observations for the embedded page engine.

use std::time::Duration;

use serde_json::{json, Value};

use super::actions::DaemonState;
use super::element::{parse_ref, resolve_element_object_id};

const INSPECT: &str = r#"function(nodes, names) {
    const node = nodes[0];
    if (!node) return { match_count: 0, element: null };
    const rect = node.getBoundingClientRect();
    const clip = value => typeof value === 'string' ? value.slice(0, 2048) : null;
    return {
        match_count: nodes.length,
        element: {
            tag: node.localName,
            text: clip(node.innerText ?? node.textContent),
            value: clip(node.value),
            attributes: Object.fromEntries(names.map(name => [name, clip(node.getAttribute(name))])),
            visible: node.checkVisibility({checkOpacity: true, checkVisibilityCSS: true}),
            enabled: !node.matches(':disabled') && !node.closest('[aria-disabled="true"], [inert]'),
            checked: typeof node.checked === 'boolean' ? node.checked : null,
            bounds: {x: rect.x, y: rect.y, width: rect.width, height: rect.height}
        }
    };
}"#;

const CONTENT: &str = r#"function(nodes, {format, limit}) {
    const node = nodes[0];
    if (!node) return null;
    const doc = node.ownerDocument || node;
    const isDocument = node === doc;
    let text;
    if (format === 'dom_html') {
        const doctype = isDocument && doc.doctype
            ? new XMLSerializer().serializeToString(doc.doctype) + '\n' : '';
        text = doctype + ((isDocument ? doc.documentElement : node)?.outerHTML || '');
    } else {
        const element = isDocument ? doc.body : node;
        text = element?.checkVisibility({checkOpacity: true, checkVisibilityCSS: true})
            ? (element.innerText ?? element.textContent ?? '') : '';
    }
    let end = Math.min(text.length, limit);
    if (end < text.length && /[\uD800-\uDBFF]/.test(text[end - 1])) end--;
    return { content: text.slice(0, end), truncated: end < text.length, url: doc.URL };
}"#;

const WAIT: &str = r#"function(nodes, {state, text, load, url}) {
    const node = nodes[0];
    if (text !== null) return (document.body?.innerText || '').includes(text);
    if (load !== null) return load === 'load'
        ? document.readyState === 'complete' : document.readyState !== 'loading';
    if (url !== null) return location.href;
    if (state === 'attached') return !!node;
    if (state === 'detached') return !node;
    const visible = nodes.some(el => el.checkVisibility({checkOpacity: true, checkVisibilityCSS: true})
        && el.getClientRects().length > 0);
    return state === 'hidden' ? !visible : visible;
}"#;

pub(super) async fn inspect(cmd: &Value, state: &DaemonState) -> Result<Value, String> {
    observe(cmd, state, INSPECT, cmd["attributes"].clone()).await
}

pub(super) async fn content(cmd: &Value, state: &DaemonState) -> Result<Value, String> {
    let value = observe(
        cmd,
        state,
        CONTENT,
        json!({"format": cmd["format"], "limit": cmd["limit"]}),
    )
    .await?;
    if value.is_null() {
        return Err("No element matched the selector for browser.content".into());
    }
    Ok(value)
}

pub(super) async fn wait(cmd: &Value, state: &DaemonState) -> Result<Value, String> {
    let args = json!({
        "state": cmd["state"].as_str().unwrap_or("visible"),
        "text": cmd["text"], "load": cmd["loadState"], "url": cmd["url"],
    });
    loop {
        let observed = observe(cmd, state, WAIT, args.clone()).await?;
        let satisfied = if let Some(pattern) = cmd["url"].as_str() {
            observed
                .as_str()
                .is_some_and(|url| super::actions::route_url_matches(pattern, url))
        } else {
            observed == Value::Bool(true)
        };
        if satisfied {
            return Ok(json!({"matched": true}));
        }
        // The embedding driver holds the deadline and drops this future on
        // timeout. Each poll sends at most one DOM observation.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn observe(
    cmd: &Value,
    state: &DaemonState,
    function: &str,
    args: Value,
) -> Result<Value, String> {
    let browser = state.browser.as_ref().ok_or("Browser not connected")?;
    let session = browser.active_session_id()?;
    let selector = cmd["selector"].as_str();
    if let Some(reference) = selector.and_then(parse_ref) {
        if state.ref_map.get(&reference).is_none() {
            return Err(format!(
                "Unknown element ref {reference}. Take a fresh browser.snapshot."
            ));
        }
        match resolve_element_object_id(
            &browser.client,
            session,
            &state.ref_map,
            &reference,
            &state.iframe_sessions,
        )
        .await
        {
            Ok((object, target_session)) => {
                let response = browser.client.send_command(
                    "Runtime.callFunctionOn",
                    Some(json!({
                        "objectId": object,
                        "functionDeclaration": format!("function(args) {{ return ({function})([this], args); }}"),
                        "arguments": [{"value": args}],
                        "returnByValue": true,
                    })),
                    Some(&target_session),
                ).await;
                let _ = browser
                    .client
                    .send_command_no_wait(
                        "Runtime.releaseObject",
                        Some(json!({"objectId": object})),
                        Some(&target_session),
                    )
                    .await;
                return result_value(response?);
            }
            Err(error) if !error.contains(" is stale") => return Err(error),
            Err(_) => {
                return match cmd["action"].as_str() {
                    Some("inspect") => Ok(json!({"match_count": 0, "element": null})),
                    Some("wait") => Ok(Value::Bool(matches!(
                        cmd["state"].as_str(),
                        Some("hidden" | "detached")
                    ))),
                    _ => Err("The element ref is stale. Take a fresh browser.snapshot.".into()),
                }
            }
        }
    }
    let nodes = if let Some(selector) = selector {
        let selector = serde_json::to_string(selector).map_err(|error| error.to_string())?;
        format!("Array.from(document.querySelectorAll({selector}))")
    } else {
        "[document]".into()
    };
    let mut params = json!({
        "expression": format!("({function})({nodes}, {args})"),
        "returnByValue": true,
    });
    if let Some(context) = cmd["uniqueContextId"].as_str() {
        params["uniqueContextId"] = json!(context);
    }
    let result = browser
        .client
        .send_command(
            "Runtime.evaluate",
            Some(params),
            Some(cmd["executionSession"].as_str().unwrap_or(session)),
        )
        .await?;
    result_value(result)
}

fn result_value(result: Value) -> Result<Value, String> {
    if let Some(exception) = result.get("exceptionDetails") {
        let message = exception
            .pointer("/exception/description")
            .and_then(Value::as_str)
            .or_else(|| exception["text"].as_str())
            .unwrap_or("DOM observation failed");
        return Err(message
            .lines()
            .next()
            .unwrap_or(message)
            .chars()
            .take(512)
            .collect());
    }
    Ok(result
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}
