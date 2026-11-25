use std::time::Duration;

use agena_testkit::{Reply, RequestMatch, TestResult};
use anyhow::ensure;
use serde_json::json;

use super::{super::fixture_without_memory, call, user};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub(super) async fn composes_session_tools_and_preserves_permissions() -> TestResult {
    fixture_without_memory()
        .configure_global(|config| config["sandbox"] = json!(false))
        .expect_plugin("agena.code_mode")
        .seed_file("workspace/source.txt", b"code mode marker\n".to_vec())
        .run(|case| async move {
            let user = user(&case).await?;
            let result = call(&user, "code_mode.execute", json!({"code": r#"
                const { tools: matches } = tools.search('fs.stat', 1);
                const size = (await tools.call(matches[0].name, {path: 'source.txt'})).payload.size;
                const text = tools.call('fs.read', {file_path: 'source.txt', mode: 'text'}).text;
                let invalid;
                try { tools.call('fs.stat', []); } catch (error) { invalid = error.code; }
                return { name: matches[0].name, marker: text.includes('code mode marker'),
                    size, invalid, standalone: typeof fetch === 'undefined' && typeof process === 'undefined'
                        && typeof __agenaInvoke === 'undefined' && this === undefined };
            "#})).await?;
            let data = result.data()?;
            ensure!(data.get("error").is_none(), "Code Mode composition failed: {data}");
            assert_eq!(data["result"]["name"], "fs.stat");
            assert_eq!(data["result"]["marker"], true);
            assert_eq!(data["result"]["size"], "code mode marker\n".len());
            assert_eq!(data["result"]["invalid"], "code_mode.invalid_call");
            assert_eq!(data["result"]["standalone"], true);
            assert_eq!(data["calls"].as_array().map(Vec::len), Some(2));
            assert_eq!(data["calls"][0]["status"], "succeeded");

            let mut policy = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
            policy["tools"]["names"]["agena.fs.write"] = json!("ask");
            policy["tools"]["names"]["fs.write"] = json!("ask");
            user.client().set_session_permission(user.id(), serde_json::from_value(policy)?).await?;
            let program = json!({"code": r#"
                return tools.call('fs.write', {path: 'composed.txt', content: 'composed marker'}).payload;
            "#});
            let denied = user.start_tool("code_mode.execute", program.clone()).await?;
            denied.wait_permission().await?.deny_once().await?;
            let denied = denied.wait_settled().await?;
            assert_eq!(denied.data()?["calls"][0]["status"], "denied");
            ensure!(denied.data()?["error"].is_string());
            let path = case.workspace().join("composed.txt");
            ensure!(!path.exists());

            let cancel_prompt = "cancel a Code Mode tool call pending approval";
            case.model()
                .expect(RequestMatch::user_text(cancel_prompt).declares_function("tools_call"))
                .reply(Reply::tool_call("cancel_code_mode", "code_mode.execute", program.clone()));
            let cancelled = user.send(cancel_prompt).await?;
            cancelled.wait_permission().await?;
            cancelled.cancel().await?;
            cancelled.wait_settled().await?.assert_cancelled()?;
            ensure!(!path.exists());

            let approved = user.start_tool("code_mode.execute", program).await?;
            let permission = approved.wait_permission().await?;
            tokio::time::sleep(Duration::from_millis(2200)).await;
            permission.allow_once().await?;
            let approved = approved.wait_settled().await?;
            assert_eq!(approved.data()?["calls"][0]["status"], "succeeded");
            assert_eq!(tokio::fs::read_to_string(path).await?, "composed marker");

            let infinite = call(&user, "code_mode.execute", json!({"code": "while (true) {}"})).await?;
            ensure!(infinite.data()?["error"].as_str().is_some_and(|error| error.contains("compute budget")));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounds_javascript_and_preserves_result_contract() -> TestResult {
    fixture_without_memory()
        .configure_global(|config| config["sandbox"] = json!(false))
        .expect_plugin("agena.code_mode")
        .run(|case| async move {
            let user = user(&case).await?;
            for (code, expected) in [
                ("return new Promise(() => {});", "unresolved promise"),
                (
                    "return new ArrayBuffer(32 * 1024 * 1024).byteLength;",
                    "larger than this engine will allocate",
                ),
                ("return Array(1_000_000_000).join('');", "compute budget"),
            ] {
                let result = call(&user, "code_mode.execute", json!({"code": code})).await?;
                let data = result.data()?;
                ensure!(
                    data["error"]
                        .as_str()
                        .is_some_and(|error| error.contains(expected)),
                    "unexpected result for {code}: {data}"
                );
            }

            let result = call(
                &user,
                "code_mode.execute",
                json!({"code": r#"
                for (let i = 0; i < 50; i++) tools.search('fs.stat', 1);
                try { tools.search('fs.stat', 1); } catch (error) { return error.code; }
            "#}),
            )
            .await?;
            assert_eq!(result.data()?["result"], "code_mode.request_limit");

            let result = call(
                &user,
                "code_mode.execute",
                json!({"code": "return '你'.repeat(6000);"}),
            )
            .await?;
            let data = result.data()?;
            assert_eq!(data["truncated"], true);
            ensure!(
                data["result"]
                    .as_str()
                    .is_some_and(|preview| preview.starts_with("\"你"))
            );
            assert!(serde_json::to_vec(&data["result"])?.len() <= 16 * 1024);
            Ok(())
        })
        .await
}
