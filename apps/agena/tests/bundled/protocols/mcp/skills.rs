use std::{
    fmt::Write as _,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use base64::Engine as _;
use sha2::{Digest, Sha256};

use super::*;

const EXTENSION: &str = "io.modelcontextprotocol/skills";
const SERVICE: &str = "plugin:claude.skills-fixture:catalog";
const SKILL: &str = "skill://review-checklist/SKILL.md";
const FILE: &str = "skill://review-checklist/references/CHECKLIST.md";
const DIRECTORY: &str = "skill://review-checklist/references";
const DOCUMENT: &str = "---\nname: review-checklist\ndescription: Review a document.\nallowed-tools: Bash\n---\nUse the fixture checklist.\n";
const SUPPORT: &str = "Fixture checklist supporting text.\n";
const TAMPERED: &str = "Fixture checklist UNVERIFIED text.\n";

#[derive(Default)]
struct SkillPeer {
    dynamic: bool,
    reads: AtomicUsize,
    version_changed: AtomicBool,
    tampered: AtomicBool,
    incorrect_frontmatter: AtomicBool,
    incomplete_catalog: AtomicBool,
    different_resource: AtomicBool,
}

fn manifest_entry(uri: &str, text: &str) -> Value {
    let mut digest = String::from("sha256:");
    for byte in Sha256::digest(text) {
        write!(digest, "{byte:02x}").unwrap();
    }
    json!({"uri":uri, "digest":digest, "size":text.len()})
}

async fn skill_peer(State(peer): State<Arc<SkillPeer>>, Json(request): Json<Value>) -> Response {
    if request.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let skill = || {
        let description = if peer.incorrect_frontmatter.load(Ordering::Relaxed) {
            "Different metadata"
        } else {
            "Review a document."
        };
        let support = if peer.version_changed.load(Ordering::Relaxed) {
            "New advertised bytes"
        } else {
            SUPPORT
        };
        let resources = if peer.dynamic {
            json!("dynamic")
        } else {
            json!([
                manifest_entry(SKILL, DOCUMENT),
                manifest_entry(FILE, support)
            ])
        };
        json!({
            "uri": SKILL,
            "frontmatter": {"description":description,"name":"review-checklist","allowed-tools":"Bash"},
            "resources": resources,
        })
    };
    let params = &request["params"];
    let declares_skills = || {
        params["_meta"]["io.modelcontextprotocol/clientCapabilities"]["extensions"][EXTENSION]
            .is_object()
    };
    let mut result = match request["method"].as_str() {
        Some("server/discover") => json!({
            "supportedVersions":["2026-07-28"],
            "capabilities":{"resources":{},"extensions":{(EXTENSION):{"directoryRead":true}}},
        }),
        Some("skills/list") if declares_skills() && params.get("cursor").is_none() => {
            // Include protocol metadata to catch untagged tool-result decoding.
            json!({"skills":[],"nextCursor":"page-two","_meta":{"fixture/catalog":true}})
        }
        Some("skills/list") if declares_skills() && params["cursor"] == "page-two" => {
            json!({"skills":[skill()]})
        }
        Some("skills/get") if declares_skills() && params["uri"] == SKILL => {
            json!({"skill":skill(),"_meta":{"fixture/catalog":true}})
        }
        Some("resources/directory/read") if params["uri"] == DIRECTORY => {
            json!({"resources":[{"uri":FILE,"name":"CHECKLIST.md","mimeType":"text/markdown"}]})
        }
        Some("resources/read") if params["uri"] == SKILL => {
            peer.reads.fetch_add(1, Ordering::Relaxed);
            let blob = base64::engine::general_purpose::STANDARD.encode(DOCUMENT);
            json!({"contents":[{"uri":SKILL,"mimeType":"text/markdown","blob":blob}]})
        }
        Some("resources/read") if params["uri"] == FILE => {
            peer.reads.fetch_add(1, Ordering::Relaxed);
            let text = if peer.tampered.load(Ordering::Relaxed) {
                TAMPERED
            } else {
                SUPPORT
            };
            let uri = if peer.different_resource.load(Ordering::Relaxed) {
                SKILL
            } else {
                FILE
            };
            json!({"contents":[{"uri":uri,"mimeType":"text/markdown","text":text}]})
        }
        _ => {
            return Json(json!({
                "jsonrpc":"2.0","id":request["id"],
                "error":{"code":-32601,"message":"Fixture method unavailable"},
            }))
            .into_response();
        }
    };
    result["resultType"] =
        if peer.incomplete_catalog.load(Ordering::Relaxed) && request["method"] == "skills/list" {
            json!("unrecognized")
        } else {
            json!("complete")
        };
    result["ttlMs"] = json!(0);
    result["cacheScope"] = json!("private");
    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response()
}

async fn answer_skill_read(run: &agena_testkit::tool::ToolRun, label: &str) -> TestResult {
    let pending = run.wait_user_input().await?;
    let input = pending
        .request
        .request
        .as_user_input()
        .context("expected skill approval")?;
    ensure!(input.questions[0].question.contains(SERVICE));
    ensure!(input.questions[0].question.contains(SKILL));
    pending
        .answer(BTreeMap::from([("0".into(), vec![label.to_owned()])]))
        .await?;
    Ok(())
}

fn skills_fixture(server_state: Arc<SkillPeer>) -> agena_testkit::TestAgenaBuilder {
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .builtin_plugin(
            "agena.mcp",
            json!({"runtime":{"reconnect":{"enabled":false}}}),
        )
        .expect_plugin("claude.skills-fixture")
        .before_start(move |case| async move {
            let peer = case
                .serve_http_on(
                    native_fixture_address()?,
                    Router::new()
                        .route("/mcp", post(skill_peer))
                        .with_state(server_state),
                )
                .await?;
            let plugin = case.paths().root.join("skills-plugin");
            tokio::fs::create_dir_all(plugin.join(".claude-plugin")).await?;
            tokio::fs::write(
                plugin.join(".claude-plugin/plugin.json"),
                br#"{"name":"skills-fixture"}"#,
            )
            .await?;
            tokio::fs::write(
                plugin.join(".mcp.json"),
                serde_json::to_vec(&json!({
                    "mcpServers":{"catalog":{"type":"http","url":peer.url("/mcp")}},
                }))?,
            )
            .await?;
            case.run_cli([
                "plugin",
                "install",
                plugin.to_str().context("fixture path")?,
            ])
            .await?
            .require_success()?;
            Ok(())
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_mcp_skills_discover_and_verify_remote_files_after_consent() -> TestResult {
    let peer = Arc::new(SkillPeer::default());
    skills_fixture(peer.clone())
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let list = user
                .call_tool("mcp.skills.list", json!({"server":SERVICE}))
                .await?;
            list.assert_succeeded()?;
            assert_eq!(list.data()?["next_cursor"], "page-two");
            let list = user
                .call_tool(
                    "mcp.skills.list",
                    json!({"server":SERVICE,"cursor":"page-two"}),
                )
                .await?;
            list.assert_succeeded()?;
            assert_eq!(list.data()?["skills"][0]["uri"], SKILL);
            let directory = user
                .call_tool(
                    "mcp.resources.directory.list",
                    json!({"server":SERVICE,"uri":DIRECTORY}),
                )
                .await?;
            directory.assert_succeeded()?;
            assert_eq!(directory.data()?["resources"][0]["uri"], FILE);
            assert_eq!(peer.reads.load(Ordering::Relaxed), 0);

            let input = json!({"server":SERVICE,"uri":SKILL});
            let denied = user.start_tool("mcp.skills.read", input.clone()).await?;
            answer_skill_read(&denied, "Decline").await?;
            denied.wait_settled().await?.assert_failed()?;
            assert_eq!(peer.reads.load(Ordering::Relaxed), 0);

            let read = user.start_tool("mcp.skills.read", input.clone()).await?;
            answer_skill_read(&read, "Allow once").await?;
            let result = read.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_text_contains("Use the fixture checklist.")?;
            let version = &result.data()?["version"];
            assert!(version.is_string());
            let supporting =
                json!({"server":SERVICE,"uri":SKILL,"file_uri":FILE,"version":version});
            let read = user
                .start_tool("mcp.skills.read", supporting.clone())
                .await?;
            answer_skill_read(&read, "Allow once").await?;
            read.wait_settled()
                .await?
                .assert_text_contains("Fixture checklist supporting text.")?;

            assert_eq!(TAMPERED.len(), SUPPORT.len());
            peer.tampered.store(true, Ordering::Relaxed);
            let read = user
                .start_tool("mcp.skills.read", supporting.clone())
                .await?;
            answer_skill_read(&read, "Allow once").await?;
            let result = read.wait_settled().await?;
            result.assert_failed()?;
            assert!(!result.output.to_string().contains("UNVERIFIED"));

            peer.version_changed.store(true, Ordering::Relaxed);
            user.call_tool("mcp.skills.read", supporting)
                .await?
                .assert_failed()?;
            assert_eq!(peer.reads.load(Ordering::Relaxed), 3);

            peer.incorrect_frontmatter.store(true, Ordering::Relaxed);
            let read = user.start_tool("mcp.skills.read", input).await?;
            answer_skill_read(&read, "Allow once").await?;
            read.wait_settled().await?.assert_failed()?;
            assert_eq!(peer.reads.load(Ordering::Relaxed), 4);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_mcp_dynamic_skills_read_each_file_with_fresh_consent() -> TestResult {
    let peer = Arc::new(SkillPeer {
        dynamic: true,
        ..Default::default()
    });
    skills_fixture(peer.clone())
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let list = json!({"server":SERVICE,"cursor":"page-two"});
            let catalog = user.call_tool("mcp.skills.list", list.clone()).await?;
            catalog.assert_succeeded()?;
            assert_eq!(catalog.data()?["skills"][0]["dynamic"], true);
            assert!(catalog.data()?["skills"][0]["file_count"].is_null());
            peer.incomplete_catalog.store(true, Ordering::Relaxed);
            user.call_tool("mcp.skills.list", list)
                .await?
                .assert_failed()?;
            let input = json!({"server":SERVICE,"uri":SKILL});
            let read = user.start_tool("mcp.skills.read", input.clone()).await?;
            answer_skill_read(&read, "Allow once").await?;
            let document = read.wait_settled().await?;
            document.assert_succeeded()?;
            document.assert_text_contains("Use the fixture checklist.")?;
            assert_eq!(document.data()?["dynamic"], true);
            assert!(document.data()?["version"].is_null());
            assert!(document.data()?["files"].is_null());

            let directory = user
                .call_tool(
                    "mcp.resources.directory.list",
                    json!({"server":SERVICE,"uri":DIRECTORY}),
                )
                .await?;
            directory.assert_succeeded()?;
            assert_eq!(directory.data()?["resources"][0]["uri"], FILE);
            let supporting = json!({"server":SERVICE,"uri":SKILL,"file_uri":FILE});
            let denied = user
                .start_tool("mcp.skills.read", supporting.clone())
                .await?;
            answer_skill_read(&denied, "Decline").await?;
            denied.wait_settled().await?.assert_failed()?;
            assert_eq!(peer.reads.load(Ordering::Relaxed), 1);

            for (changed, expected) in [(false, SUPPORT), (true, TAMPERED)] {
                peer.tampered.store(changed, Ordering::Relaxed);
                let read = user
                    .start_tool("mcp.skills.read", supporting.clone())
                    .await?;
                answer_skill_read(&read, "Allow once").await?;
                let result = read.wait_settled().await?;
                result.assert_succeeded()?;
                assert_eq!(result.data()?["contents"][0]["text"], expected);
            }
            for uri in [
                "skill://another/file.md",
                "skill://review-checklist/references/%2E%2E/file.md",
            ] {
                user.call_tool(
                    "mcp.skills.read",
                    json!({"server":SERVICE,"uri":SKILL,"file_uri":uri}),
                )
                .await?
                .assert_failed()?;
            }
            assert_eq!(peer.reads.load(Ordering::Relaxed), 3);

            peer.different_resource.store(true, Ordering::Relaxed);
            let read = user.start_tool("mcp.skills.read", supporting).await?;
            answer_skill_read(&read, "Allow once").await?;
            read.wait_settled().await?.assert_failed()?;
            peer.incorrect_frontmatter.store(true, Ordering::Relaxed);
            let read = user.start_tool("mcp.skills.read", input).await?;
            answer_skill_read(&read, "Allow once").await?;
            read.wait_settled().await?.assert_failed()?;
            assert_eq!(peer.reads.load(Ordering::Relaxed), 5);
            Ok(())
        })
        .await
}
