use super::*;

const PEER_CODE: &str = r#"
import json
import sys
import uuid

instance = str(uuid.uuid4())
with open(sys.argv[1], "a", encoding="utf-8") as launches:
    launches.write(instance + "\n")

uri = "ui://fixture/app"
value = None

def tool(name, visibility):
    return {
        "name": name, "inputSchema": {"type": "object"},
        "_meta": {"ui": {"resourceUri": uri, "visibility": [visibility]}},
    }

for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    name = request["method"]
    params = request.get("params") or {}
    reply = {"jsonrpc": "2.0", "id": request["id"]}
    if name == "initialize":
        reply["result"] = {
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}, "resources": {}},
            "serverInfo": {"name": "fixture-stdio-app", "version": "1"},
        }
    elif name == "tools/list":
        reply["result"] = {"tools": [tool("render", "model"), tool("increment", "app")]}
    elif name == "tools/call" and params["name"] == "render":
        value = params["arguments"]["selection"]
        reply["result"] = {"content": [{"type": "text", "text": "App ready"}],
                           "structuredContent": {"instance": instance}}
    elif name == "tools/call" and params["name"] == "increment" and value is not None:
        value += 1
        reply["result"] = {"content": [], "structuredContent": {"instance": instance, "value": value}}
    elif name == "resources/read" and params["uri"] == uri and value is not None:
        reply["result"] = {"contents": [{
            "uri": uri, "mimeType": "text/html;profile=mcp-app",
            "text": "<main>Selection " + str(value) + "</main>",
        }]}
    else:
        reply["error"] = {"code": -32601, "message": "No App on this process"}
    print(json.dumps(reply), flush=True)
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stdio_app_borrows_only_the_original_session_process() -> TestResult {
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("agena.mcp")
        .before_start(|case| async move {
            let launches = case.paths().root.join("mcp-app-launches");
            let mut config: Value = serde_json::from_slice(
                &tokio::fs::read(&case.paths().global_config).await?,
            )?;
            config["plugins"]["list"]["agena.mcp"] = json!({
                "enabled": true, "package": {"kind": "static"},
                "settings": {
                    "runtime": {"reconnect": {"enabled": false}},
                    "servers": {(SERVER): {
                        "transport": "stdio",
                        "process": {"command": "python3", "args": ["-c", PEER_CODE, launches.to_str().context("launch log path")?]},
                    }},
                },
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .run(|case| async move {
            super::super::super::cordis::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let launch_log = case.paths().root.join("mcp-app-launches");
            let http = case.http().await;
            let render = |selection| user.call_tool("mcp.tools.call", json!({
                "server": SERVER, "name": "render", "arguments": {"selection": selection},
            }));
            let rendered = render(41).await?;
            rendered.assert_succeeded()?;
            let instance = rendered.data()?["structured_content"]["instance"]
                .as_str().context("renderer instance")?;
            let session = user.id();
            let part = rendered.part.part_id;
            let path = format!("/api/v1/sessions/{session}/parts/{part}/mcp-app");
            let document: Value = http.get(&path).await?.require_success()?.json()?;
            assert_eq!(document["html"], "<main>Selection 41</main>");
            let revision = document["revision"].as_str().context("MCP App revision")?;
            let action = || -> anyhow::Result<_> {
                Ok(http.request(Method::POST, &path)?
                    .header("if-match", format!("\"{revision}\""))
                    .json(&json!({"method": "tools/call", "params": {"name":"increment"}})))
            };
            let changed: Value = http.execute(action()?).await?.require_success()?.json()?;
            assert_eq!(changed["structuredContent"], json!({"instance": instance, "value": 42}));
            assert_eq!(tokio::fs::read_to_string(&launch_log).await?.trim(), instance);

            let other = case.user().new_session().await?;
            assert_eq!(http.get(&format!("/api/v1/sessions/{}/parts/{part}/mcp-app", other.id())).await?.status, StatusCode::NOT_FOUND);
            let extra = case.paths().root.join("another App directory");
            tokio::fs::create_dir_all(&extra).await?;
            assert!(user.client().register_working_directory(session, extra).await?.added);
            assert_eq!(http.execute(action()?).await?.status, StatusCode::CONFLICT);
            assert_eq!(tokio::fs::read_to_string(&launch_log).await?.trim(), instance);

            let replacement = render(53).await?;
            replacement.assert_succeeded()?;
            let new_instance = replacement.data()?["structured_content"]["instance"]
                .as_str().context("replacement instance")?;
            assert_ne!(new_instance, instance);
            let replacement_path = format!(
                "/api/v1/sessions/{session}/parts/{}/mcp-app", replacement.part.part_id
            );
            let document: Value = http.get(&replacement_path).await?.require_success()?.json()?;
            assert_eq!(document["html"], "<main>Selection 53</main>");
            assert_eq!(http.get(&path).await?.status, StatusCode::CONFLICT);

            case.settings().reload_and_wait().await?;
            assert_eq!(http.get(&replacement_path).await?.status, StatusCode::CONFLICT);
            assert_eq!(tokio::fs::read_to_string(&launch_log).await?.lines().count(), 2);
            Ok(())
        }).await
}
