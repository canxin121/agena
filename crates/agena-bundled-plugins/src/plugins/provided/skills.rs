//! `agena.skills` — discovery and activation runtime for bundled, user and
//! workspace Skill packages.
//!
//! Packaged skills from `agena-skills` are also projected here so a fresh
//! install has workflow-like tools before any user-defined content exists.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::{collections::BTreeMap, fmt};

use agena_macros::ToolInput;
use agena_skills::discovery::{
    DiscoveryDiagnostic, default_command_roots, default_roots, scan_commands_with_diagnostics,
    scan_with_diagnostics,
};
use agena_skills::skill::{Skill, SkillDependencies, SkillFrontmatter};
use globset::{Glob, GlobSetBuilder};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{
    AskUserOption, AskUserQuestion, AskUserRequest, HostSetSessionModelRequest,
    HostStorageDeleteRequest, HostStorageGetRequest, HostStorageScope, HostStorageSetRequest,
};
use agena_plugin_host::sdk::{
    ChatSystemTransformInput, ChatSystemTransformPatch, HostCapability, HostClient, InitContext,
    InitOutcome, Result as SdkResult, SessionEndInput, ToolBeforeInput, ToolBeforePatch,
    ToolDefinitionInput, ToolDefinitionPatch, ToolInvokeContext, ToolInvokeOutput,
    UserPromptSubmitInput, UserPromptSubmitPatch,
};

pub(crate) const SKILLS_PLUGIN_ID: &str = "agena.skills";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoveredToolKind {
    Skill,
    Command,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use agena_plugin_host::sdk::host_api::{
        AskUserResponse, HostSession, HostSetSessionModelRequest, HostSetSessionModelResponse,
        HostStorageDeleteRequest, HostStorageGetRequest, HostStorageGetResponse,
        HostStorageSetRequest,
    };
    use agena_plugin_host::sdk::{
        HookSubscription, HostClient, InitContext, Plugin, PluginKey, PluginSkillDefinition,
        ToolBeforeInput, ToolInvokeContext, UserPromptSubmitInput,
    };

    use super::{ActiveSkill, SkillsPlugin, SkillsRefreshInput, SkillsRunInput};

    #[derive(Default)]
    struct RecordingHost {
        models: Mutex<Vec<HostSetSessionModelRequest>>,
        trust: Mutex<BTreeMap<String, String>>,
        trust_prompts: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl HostClient for RecordingHost {
        async fn log(
            &self,
            _level: agena_plugin_host::sdk::host_api::LogLevel,
            _message: String,
            _fields: serde_json::Value,
        ) {
        }

        async fn publish_event(
            &self,
            _event: agena_plugin_host::sdk::EventEnvelope,
        ) -> agena_plugin_host::sdk::Result<()> {
            Ok(())
        }

        async fn subscribe_events(
            &self,
            _filter: agena_plugin_host::sdk::EventFilter,
        ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::host_api::EventSubscription>
        {
            Ok(agena_plugin_host::sdk::host_api::EventSubscription {
                id: "test".to_string(),
            })
        }

        async fn ask_permission(
            &self,
            _request: agena_plugin_host::sdk::PermissionAskInput,
        ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::PermissionDecision> {
            Ok(agena_plugin_host::sdk::PermissionDecision::Deny)
        }

        async fn read_config(
            &self,
            _path: Option<String>,
        ) -> agena_plugin_host::sdk::Result<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }

        async fn invoke_tool(
            &self,
            _tool: String,
            _input: serde_json::Value,
        ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
            Ok(agena_plugin_host::sdk::ToolInvokeOutput::text("test"))
        }

        async fn ask_user(
            &self,
            _request: agena_plugin_host::sdk::host_api::AskUserRequest,
        ) -> agena_plugin_host::sdk::Result<AskUserResponse> {
            self.trust_prompts.fetch_add(1, Ordering::Relaxed);
            Ok(AskUserResponse {
                answers: BTreeMap::from([(
                    "trust".to_string(),
                    vec!["Trust and activate".to_string()],
                )]),
                ..AskUserResponse::default()
            })
        }

        async fn set_session_model(
            &self,
            request: HostSetSessionModelRequest,
        ) -> agena_plugin_host::sdk::Result<HostSetSessionModelResponse> {
            self.models
                .lock()
                .expect("recording host lock")
                .push(request.clone());
            Ok(HostSetSessionModelResponse {
                session: HostSession {
                    id: request.session_id.unwrap_or_default(),
                    parent_id: None,
                    root_id: 1,
                    workspace_id: 1,
                    title: "test".to_string(),
                    is_subagent: false,
                },
                provider_id: "fixture".to_string(),
                adapter_id: None,
                model_id: "model".to_string(),
            })
        }

        async fn storage_get(
            &self,
            request: HostStorageGetRequest,
        ) -> agena_plugin_host::sdk::Result<HostStorageGetResponse> {
            Ok(HostStorageGetResponse {
                value: self
                    .trust
                    .lock()
                    .expect("trust storage lock")
                    .get(request.key.as_str())
                    .cloned(),
            })
        }

        async fn storage_set(
            &self,
            request: HostStorageSetRequest,
        ) -> agena_plugin_host::sdk::Result<()> {
            self.trust
                .lock()
                .expect("trust storage lock")
                .insert(request.key, request.value);
            Ok(())
        }

        async fn storage_delete(
            &self,
            request: HostStorageDeleteRequest,
        ) -> agena_plugin_host::sdk::Result<()> {
            self.trust
                .lock()
                .expect("trust storage lock")
                .remove(request.key.as_str());
            Ok(())
        }
    }

    async fn init_test_plugin(
        plugin: &SkillsPlugin,
        workspace_root: &std::path::Path,
        host: Arc<RecordingHost>,
    ) {
        init_test_plugin_with_config(plugin, workspace_root, host, serde_json::Value::Null).await;
    }

    async fn init_test_plugin_with_config(
        plugin: &SkillsPlugin,
        workspace_root: &std::path::Path,
        host: Arc<RecordingHost>,
        config: serde_json::Value,
    ) {
        plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: workspace_root.to_path_buf(),
                    plugin_id: PluginKey::new("agena", "skills").expect("plugin key"),
                    host_callback_url: None,
                    host_callback_token: None,
                    config,
                    protocol_version: 1,
                },
                host,
            )
            .await
            .expect("init plugin");
    }

    #[test]
    fn manifest_exposes_activation_lifecycle_tools_and_hooks() {
        let manifest = SkillsPlugin::new().manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            tool_names,
            [
                "list",
                "get",
                "run",
                "read_resource",
                "refresh",
                "status",
                "deactivate"
            ]
        );
        assert!(
            manifest
                .hooks
                .contains(HookSubscription::CHAT_SYSTEM_TRANSFORM)
        );
        assert!(manifest.hooks.contains(HookSubscription::TOOL_BEFORE));
        assert!(
            manifest
                .hooks
                .contains(HookSubscription::USER_PROMPT_SUBMIT)
        );
    }

    #[test]
    fn bundled_aliases_resolve_to_canonical_names() {
        let dir = tempfile::tempdir().expect("temp dir");
        let catalog = SkillsPlugin::discovered_tools_for_workspace(dir.path());
        let (name, _) =
            SkillsPlugin::resolve_tool(&catalog.tools, "bootstrap").expect("resolve bundled alias");
        assert_eq!(name, "init");
    }

    #[test]
    fn watcher_uses_recursive_roots_and_nonrecursive_existing_parents() {
        let workspace = tempfile::tempdir().expect("workspace");
        let existing = workspace.path().join("skills");
        std::fs::create_dir_all(&existing).expect("existing root");
        let (recursive, recursive_mode) = super::watcher_target(existing.as_path());
        assert_eq!(recursive, existing);
        assert!(matches!(recursive_mode, notify::RecursiveMode::Recursive));

        let missing = workspace.path().join(".agena/skills");
        let (ancestor, ancestor_mode) = super::watcher_target(missing.as_path());
        assert_eq!(ancestor, workspace.path());
        assert!(matches!(ancestor_mode, notify::RecursiveMode::NonRecursive));
    }

    #[test]
    fn active_skill_allowlist_accepts_canonical_compact_and_bare_names() {
        let active = ActiveSkill {
            canonical_name: "demo".to_string(),
            prompt: String::new(),
            allowed_tools: vec![
                "agena.fs.read".to_string(),
                "shell.run".to_string(),
                "grep".to_string(),
            ],
            model: None,
            source_path: None,
            source: "bundled".to_string(),
            content_hash: "hash".to_string(),
        };
        let input = |tool: &str| ToolBeforeInput {
            tool: tool.parse().expect("tool key"),
            session_id: 1,
            call_id: 1,
            workspace_root: "/workspace".to_string(),
            tags: Vec::new(),
            input: serde_json::Value::Null,
            title_override: None,
            metadata: std::collections::BTreeMap::new(),
        };
        assert!(SkillsPlugin::tool_is_allowed(
            &active,
            &input("agena.fs.read")
        ));
        assert!(SkillsPlugin::tool_is_allowed(
            &active,
            &input("agena.shell.run")
        ));
        assert!(SkillsPlugin::tool_is_allowed(
            &active,
            &input("agena.fs.grep")
        ));
        assert!(!SkillsPlugin::tool_is_allowed(
            &active,
            &input("agena.fs.apply_patch")
        ));
        assert!(SkillsPlugin::tool_is_allowed(
            &active,
            &input("agena.skills.deactivate")
        ));
        assert!(SkillsPlugin::tool_is_allowed(
            &active,
            &input("agena.tools.call")
        ));
    }

    #[tokio::test]
    async fn activation_applies_a_qualified_model_to_the_session() {
        let workspace = tempfile::tempdir().expect("workspace");
        let skill_dir = workspace.path().join(".agena/skills/model-switch");
        std::fs::create_dir_all(&skill_dir).expect("skill directory");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: model_switch\ndescription: model test\nmodel: fixture/model\n---\nUse the selected model.",
        )
        .expect("skill file");

        let plugin = SkillsPlugin::new();
        let host = Arc::new(RecordingHost::default());
        plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: workspace.path().to_path_buf(),
                    plugin_id: PluginKey::new("agena", "skills").expect("plugin key"),
                    host_callback_url: None,
                    host_callback_token: None,
                    config: serde_json::Value::Null,
                    protocol_version: 1,
                },
                host.clone(),
            )
            .await
            .expect("init plugin");
        let root = workspace.path().display().to_string();
        let output = plugin
            .invoke_run(
                &SkillsRunInput {
                    name: "model_switch".to_string(),
                    args: None,
                },
                &ToolInvokeContext {
                    tool_name: "run",
                    session_id: 42,
                    call_id: 1,
                    workspace_root: root.as_str(),
                },
            )
            .await
            .expect("activate skill");

        assert!(
            output
                .output_text
                .contains("Applied session model: fixture/model")
        );
        assert_eq!(
            host.models.lock().expect("recorded model").as_slice(),
            [HostSetSessionModelRequest {
                session_id: Some(42),
                model: "fixture/model".to_string(),
            }]
        );
    }

    #[tokio::test]
    async fn exact_hash_trust_survives_plugin_reconstruction() {
        let workspace = tempfile::tempdir().expect("workspace");
        let skill_dir = workspace.path().join(".agena/skills/persisted-trust");
        std::fs::create_dir_all(&skill_dir).expect("skill directory");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: persisted_trust\ndescription: trust test\n---\nUse safe instructions.",
        )
        .expect("skill file");
        let host = Arc::new(RecordingHost::default());
        let root = workspace.path().display().to_string();

        for call_id in [1_i64, 2_i64] {
            let plugin = SkillsPlugin::new();
            plugin
                .init(
                    InitContext {
                        agena_version: "test".to_string(),
                        workspace_root: workspace.path().to_path_buf(),
                        plugin_id: PluginKey::new("agena", "skills").expect("plugin key"),
                        host_callback_url: None,
                        host_callback_token: None,
                        config: serde_json::Value::Null,
                        protocol_version: 1,
                    },
                    host.clone(),
                )
                .await
                .expect("init plugin");
            plugin
                .invoke_run(
                    &SkillsRunInput {
                        name: "persisted_trust".to_string(),
                        args: None,
                    },
                    &ToolInvokeContext {
                        tool_name: "run",
                        session_id: 42,
                        call_id,
                        workspace_root: root.as_str(),
                    },
                )
                .await
                .expect("activate trusted skill");
        }

        assert_eq!(host.trust_prompts.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn active_skill_is_restored_after_reconstruction_and_discarded_when_changed() {
        let workspace = tempfile::tempdir().expect("workspace");
        let skill_dir = workspace.path().join(".agena/skills/persisted-active");
        std::fs::create_dir_all(&skill_dir).expect("skill directory");
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_path,
            "---\nname: persisted_active\ndescription: active persistence test\nallowed-tools:\n  - shell.run\n---\nFirst revision instructions.",
        )
        .expect("skill file");
        let host = Arc::new(RecordingHost::default());
        let root = workspace.path().display().to_string();
        let context = ToolInvokeContext {
            tool_name: "run",
            session_id: 42,
            call_id: 1,
            workspace_root: root.as_str(),
        };

        let original = SkillsPlugin::new();
        init_test_plugin(&original, workspace.path(), host.clone()).await;
        original
            .invoke_run(
                &SkillsRunInput {
                    name: "persisted_active".to_string(),
                    args: Some("with arguments".to_string()),
                },
                &context,
            )
            .await
            .expect("activate skill");

        let reconstructed = SkillsPlugin::new();
        init_test_plugin(&reconstructed, workspace.path(), host.clone()).await;
        let restored = reconstructed
            .invoke_status(&context)
            .await
            .expect("restore active skill");
        assert!(
            restored
                .output_text
                .contains("Active skill: persisted_active")
        );
        let restored_before = reconstructed
            .tool_execute_before(ToolBeforeInput {
                tool: "agena.fs.read".parse().expect("tool key"),
                session_id: 42,
                call_id: 2,
                workspace_root: root.clone(),
                tags: Vec::new(),
                input: serde_json::Value::Null,
                title_override: None,
                metadata: BTreeMap::new(),
            })
            .await
            .expect("enforce restored allowlist");
        assert!(
            restored_before
                .and_then(|patch| patch.abort_reason)
                .is_some_and(|reason| reason.contains("persisted_active"))
        );

        std::fs::write(
            &skill_path,
            "---\nname: persisted_active\ndescription: active persistence test\nallowed-tools:\n  - fs.read\n---\nSecond revision instructions.",
        )
        .expect("change skill file");
        let after_change = SkillsPlugin::new();
        init_test_plugin(&after_change, workspace.path(), host.clone()).await;
        let inactive = after_change
            .invoke_status(&context)
            .await
            .expect("discard stale active skill");
        assert!(inactive.output_text.contains("No skill is active"));
        assert!(
            host.trust
                .lock()
                .expect("trust storage lock")
                .get("active")
                .is_none()
        );
    }

    #[tokio::test]
    async fn skills_config_filters_disabled_names_and_accepts_extra_workspace_roots() {
        let workspace = tempfile::tempdir().expect("workspace");
        let disabled_dir = workspace.path().join(".agena/skills/disabled-skill");
        let extra_dir = workspace.path().join("contributed-skills/extra-skill");
        std::fs::create_dir_all(&disabled_dir).expect("disabled skill dir");
        std::fs::create_dir_all(&extra_dir).expect("extra skill dir");
        std::fs::write(
            disabled_dir.join("SKILL.md"),
            "---\nname: disabled_skill\ndescription: should be hidden\n---\nDisabled.",
        )
        .expect("disabled skill");
        std::fs::write(
            extra_dir.join("SKILL.md"),
            "---\nname: extra_skill\ndescription: explicit workspace root\n---\nExtra.",
        )
        .expect("extra skill");

        let plugin = SkillsPlugin::new();
        init_test_plugin_with_config(
            &plugin,
            workspace.path(),
            Arc::new(RecordingHost::default()),
            serde_json::json!({
                "disabled": ["disabled_skill"],
                "additional_roots": ["contributed-skills"],
            }),
        )
        .await;

        let listed = plugin
            .invoke_list(&super::SkillsListInput {
                offset: None,
                limit: None,
                kind: Some("skill".to_string()),
                verbose: false,
            })
            .await
            .expect("list configured catalog");
        let tool_names = listed
            .payload
            .as_ref()
            .and_then(|payload| payload.get("tools"))
            .and_then(serde_json::Value::as_array)
            .expect("tools payload")
            .iter()
            .filter_map(|tool| tool.get("name").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert!(tool_names.contains(&"extra_skill"));
        assert!(!tool_names.contains(&"disabled_skill"));
        assert!(
            plugin
                .invoke_get(&super::SkillsGetInput {
                    name: "disabled_skill".to_string(),
                })
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn refresh_reports_deterministic_catalog_generations() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin = SkillsPlugin::new();
        init_test_plugin(
            &plugin,
            workspace.path(),
            Arc::new(RecordingHost::default()),
        )
        .await;

        let first = plugin
            .invoke_refresh(&SkillsRefreshInput::default())
            .await
            .expect("initial refresh");
        let first_payload = first.payload.expect("initial refresh payload");
        assert_eq!(first_payload["changed"], true);
        assert_eq!(first_payload["generation"], 1);

        let unchanged = plugin
            .invoke_refresh(&SkillsRefreshInput::default())
            .await
            .expect("unchanged refresh");
        let unchanged_payload = unchanged.payload.expect("unchanged refresh payload");
        assert_eq!(unchanged_payload["changed"], false);
        assert_eq!(unchanged_payload["generation"], 1);

        let added_dir = workspace.path().join(".agena/skills/refreshed-skill");
        std::fs::create_dir_all(&added_dir).expect("added skill dir");
        std::fs::write(
            added_dir.join("SKILL.md"),
            "---\nname: refreshed_skill\ndescription: refresh test\n---\nFresh.",
        )
        .expect("added skill");
        let changed = plugin
            .invoke_refresh(&SkillsRefreshInput::default())
            .await
            .expect("changed refresh");
        let changed_payload = changed.payload.expect("changed refresh payload");
        assert_eq!(changed_payload["changed"], true);
        assert_eq!(changed_payload["generation"], 2);
        plugin
            .invoke_get(&super::SkillsGetInput {
                name: "refreshed_skill".to_string(),
            })
            .await
            .expect("newly discovered skill");
    }

    #[tokio::test]
    async fn implicit_activation_requires_trust_and_a_matching_workspace_path() {
        let workspace = tempfile::tempdir().expect("workspace");
        let skill_dir = workspace.path().join(".agena/skills/path-gated");
        std::fs::create_dir_all(&skill_dir).expect("skill directory");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: path_gated\ndescription: Path-gated implicit test\nallow-implicit-invocation: true\npaths: [src/*.rs]\nallowed-tools: [agena.fs.read]\n---\nRead the matching Rust file.",
        )
        .expect("skill file");
        let host = Arc::new(RecordingHost::default());
        let plugin = SkillsPlugin::new();
        init_test_plugin(&plugin, workspace.path(), host.clone()).await;

        // An untrusted filesystem Skill cannot be injected just because a
        // prompt happens to mention a matching path.
        assert!(
            plugin
                .user_prompt_submit(UserPromptSubmitInput {
                    session_id: 41,
                    prompt: "Please inspect src/lib.rs".to_string(),
                })
                .await
                .expect("untrusted hook")
                .is_none()
        );

        let hash = plugin
            .discovered_tools()
            .expect("catalog")
            .get("path_gated")
            .expect("skill")
            .skill
            .content_hash();
        host.trust
            .lock()
            .expect("trust storage")
            .insert(hash, "trusted".to_string());

        let patch = plugin
            .user_prompt_submit(UserPromptSubmitInput {
                session_id: 41,
                prompt: "Please inspect src/lib.rs".to_string(),
            })
            .await
            .expect("trusted hook")
            .expect("path-gated activation");
        assert!(
            patch
                .additional_context
                .as_deref()
                .is_some_and(|text| text.contains("path_gated"))
        );
        let active = plugin
            .active_skills()
            .expect("active skills")
            .get(&41)
            .cloned()
            .expect("active skill");
        assert_eq!(active.canonical_name, "path_gated");
        assert_eq!(active.model, None);

        let unrelated = plugin
            .user_prompt_submit(UserPromptSubmitInput {
                session_id: 42,
                prompt: "Please inspect docs/readme.md".to_string(),
            })
            .await
            .expect("unmatched hook");
        assert!(unrelated.is_none());
    }

    #[tokio::test]
    async fn skills_config_rejects_workspace_escape_roots() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin = SkillsPlugin::new();
        let result = plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: workspace.path().to_path_buf(),
                    plugin_id: PluginKey::new("agena", "skills").expect("plugin key"),
                    host_callback_url: None,
                    host_callback_token: None,
                    config: serde_json::json!({ "additional_roots": ["../outside"] }),
                    protocol_version: 1,
                },
                Arc::new(RecordingHost::default()),
            )
            .await;
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skills_config_rejects_existing_roots_that_symlink_outside_workspace() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let link = workspace.path().join("linked-skills");
        std::os::unix::fs::symlink(outside.path(), &link).expect("create outside symlink");
        let plugin = SkillsPlugin::new();
        let result = plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: workspace.path().to_path_buf(),
                    plugin_id: PluginKey::new("agena", "skills").expect("plugin key"),
                    host_callback_url: None,
                    host_callback_token: None,
                    config: serde_json::json!({ "additional_roots": ["linked-skills"] }),
                    protocol_version: 1,
                },
                Arc::new(RecordingHost::default()),
            )
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn plugin_manifest_skills_are_non_bundled_catalog_entries_without_resources() {
        let catalog = SkillsPlugin::plugin_contributed_tools_from_manifests(vec![(
            "example.docs".to_string(),
            "1.2.3".to_string(),
            vec![PluginSkillDefinition {
                name: "plugin_docs".to_string(),
                description: "Use the plugin documentation workflow.".to_string(),
                instructions: "Read the plugin documentation first.".to_string(),
                aliases: vec!["docs-plugin".to_string()],
                allowed_tools: vec!["agena.fs.read".to_string()],
                ..PluginSkillDefinition::default()
            }],
        )]);
        let tool = catalog
            .get("plugin_docs")
            .expect("plugin skill contribution");
        assert_eq!(tool.kind, super::DiscoveredToolKind::Skill);
        assert_eq!(tool.skill.frontmatter.aliases, ["docs-plugin"]);
        assert_eq!(tool.skill.body, "Read the plugin documentation first.");
        assert_eq!(tool.origin.source_label(), "plugin:example.docs@1.2.3");
        assert!(tool.origin.needs_activation_trust());
        assert!(!tool.origin.supports_resources());
    }
}

impl AsRef<str> for DiscoveredToolKind {
    fn as_ref(&self) -> &str {
        match self {
            Self::Skill => "skill",
            Self::Command => "command",
        }
    }
}

impl fmt::Display for DiscoveredToolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

fn watcher_target(root: &Path) -> (PathBuf, RecursiveMode) {
    if root.is_dir() {
        return (root.to_path_buf(), RecursiveMode::Recursive);
    }
    let mut ancestor = root.to_path_buf();
    while !ancestor.is_dir() {
        if !ancestor.pop() {
            return (PathBuf::from("."), RecursiveMode::NonRecursive);
        }
    }
    (ancestor, RecursiveMode::NonRecursive)
}

fn display_values(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values
            .iter()
            .map(|value| format!("`{}`", value.trim()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Clone)]
struct DiscoveredTool {
    skill: Skill,
    kind: DiscoveredToolKind,
    origin: SkillOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SkillOrigin {
    Bundled,
    Filesystem,
    Plugin {
        plugin_id: String,
        plugin_version: String,
    },
}

impl SkillOrigin {
    fn source_label(&self) -> String {
        match self {
            Self::Bundled => "bundled".to_string(),
            Self::Filesystem => "filesystem".to_string(),
            Self::Plugin {
                plugin_id,
                plugin_version,
            } => format!("plugin:{plugin_id}@{plugin_version}"),
        }
    }

    fn needs_activation_trust(&self) -> bool {
        !matches!(self, Self::Bundled)
    }

    fn supports_resources(&self) -> bool {
        matches!(self, Self::Filesystem)
    }
}

#[derive(Debug, Clone, Default)]
struct DiscoveredCatalog {
    tools: BTreeMap<String, DiscoveredTool>,
    diagnostics: Vec<DiscoveryDiagnostic>,
}

/// Plugin-owned policy for filesystem-backed Skills. The regular roots remain
/// enabled by default; extra roots are deliberately workspace-relative so a
/// project configuration cannot silently turn an arbitrary user directory
/// into model-visible prompt content.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct SkillsPluginConfig {
    /// Canonical Skill/command names hidden from discovery and activation.
    disabled: Vec<String>,
    /// Extra workspace-relative directories containing `SKILL.md` packages.
    additional_roots: Vec<PathBuf>,
    /// Extra workspace-relative directories containing slash-command `.md`
    /// files.
    additional_command_roots: Vec<PathBuf>,
    /// Conservative automatic activation policy.  A Skill must separately
    /// opt in with `allow-implicit-invocation: true` and provide at least one
    /// matching workspace-relative `paths` glob; this setting only bounds the
    /// catalog work and injected instruction size.
    implicit_invocation: SkillsImplicitInvocationConfig,
    /// Cross-platform OS watcher used only to invalidate the filesystem
    /// catalog. Discovery/trust/activation still happen at a normal request
    /// boundary; watcher events never inject instructions on their own.
    watcher: SkillsWatcherConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct SkillsImplicitInvocationConfig {
    /// The policy is enabled by default, but no shipped or discovered Skill
    /// is eligible unless its own frontmatter explicitly opts in and supplies
    /// path gates.  Setting this false disables the feature globally.
    enabled: bool,
    /// Maximum number of path-matched, trusted candidates considered per user
    /// prompt. The deterministic sort uses match score then canonical name.
    #[schemars(range(min = 1, max = 128))]
    max_candidates: u16,
    /// Maximum rendered Skill instruction size injected by automatic
    /// activation. This is a character budget (not tokenizer-specific), so
    /// the catalog cannot unexpectedly consume a model's context window.
    #[schemars(range(min = 256, max = 65536))]
    max_instruction_chars: u32,
}

impl Default for SkillsImplicitInvocationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_candidates: 32,
            max_instruction_chars: 12_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct SkillsWatcherConfig {
    enabled: bool,
}

impl Default for SkillsWatcherConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl SkillsPluginConfig {
    fn validate_for_workspace(&self, workspace_root: &Path) -> SdkResult<()> {
        if self.implicit_invocation.max_candidates == 0
            || self.implicit_invocation.max_candidates > 128
        {
            return Err(PluginError::invalid_params(
                "skills config implicit_invocation.max_candidates must be between 1 and 128",
            ));
        }
        if self.implicit_invocation.max_instruction_chars < 256
            || self.implicit_invocation.max_instruction_chars > 65_536
        {
            return Err(PluginError::invalid_params(
                "skills config implicit_invocation.max_instruction_chars must be between 256 and 65536",
            ));
        }
        let canonical_workspace = workspace_root.canonicalize().map_err(|error| {
            PluginError::invalid_params(format!(
                "skills config cannot canonicalize workspace '{}': {error}",
                workspace_root.display()
            ))
        })?;
        let mut seen = std::collections::BTreeSet::new();
        for name in &self.disabled {
            let normalized = name.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                return Err(PluginError::invalid_params(
                    "skills config disabled entries cannot be blank",
                ));
            }
            if !seen.insert(normalized) {
                return Err(PluginError::invalid_params(format!(
                    "skills config lists the same disabled name more than once: '{name}'"
                )));
            }
        }
        for (label, roots) in [
            ("additional_roots", &self.additional_roots),
            ("additional_command_roots", &self.additional_command_roots),
        ] {
            for root in roots {
                if root.as_os_str().is_empty()
                    || root.is_absolute()
                    || root
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir))
                {
                    return Err(PluginError::invalid_params(format!(
                        "skills config {label} entries must be non-empty workspace-relative paths without '..'"
                    )));
                }
                let candidate = workspace_root.join(root);
                if !candidate.starts_with(workspace_root) {
                    return Err(PluginError::invalid_params(format!(
                        "skills config {label} entry '{}' escapes the workspace",
                        root.display()
                    )));
                }
                // A lexical workspace-relative path may still resolve through
                // a symlink outside the workspace. Existing roots are
                // canonicalized at config-load time so that cannot silently
                // turn an arbitrary user directory into model-visible Skill
                // instructions. A non-existent root is harmless until a
                // later config reload validates it again.
                if candidate.exists() {
                    let canonical_candidate = candidate.canonicalize().map_err(|error| {
                        PluginError::invalid_params(format!(
                            "skills config cannot canonicalize {label} entry '{}': {error}",
                            root.display()
                        ))
                    })?;
                    if !canonical_candidate.starts_with(&canonical_workspace) {
                        return Err(PluginError::invalid_params(format!(
                            "skills config {label} entry '{}' resolves outside the workspace",
                            root.display()
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn disabled_name(&self, name: &str) -> bool {
        self.disabled
            .iter()
            .any(|disabled| disabled.trim().eq_ignore_ascii_case(name.trim()))
    }
}

#[derive(Debug, Clone)]
struct CatalogRefresh {
    catalog: DiscoveredCatalog,
    fingerprint: String,
    generation: u64,
    changed: bool,
}

#[derive(Debug, Clone, Default)]
struct CatalogState {
    fingerprint: Option<String>,
    generation: u64,
}

/// Owns the platform watcher for as long as the static Skills plugin lives.
/// `notify` invokes the callback on its own worker context; an atomic counter
/// is sufficient because events only invalidate the next discovery result and
/// never carry content into the model context.
struct SkillCatalogWatcher {
    _watcher: RecommendedWatcher,
    watched_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveSkill {
    canonical_name: String,
    prompt: String,
    allowed_tools: Vec<String>,
    model: Option<String>,
    source_path: Option<PathBuf>,
    #[serde(default)]
    source: String,
    content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct SkillsListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default)]
    verbose: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("name"), non_empty("name"))]
#[serde(deny_unknown_fields)]
struct SkillsGetInput {
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("name", "args"), non_empty("name"))]
#[serde(deny_unknown_fields)]
struct SkillsRunInput {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    args: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("name", "path"), non_empty("name", "path"))]
#[serde(deny_unknown_fields)]
struct SkillsReadResourceInput {
    name: String,
    path: String,
    #[serde(default = "default_resource_limit")]
    #[schemars(range(min = 1, max = 1048576))]
    max_bytes: u32,
}

const fn default_resource_limit() -> u32 {
    262_144
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct SkillsDeactivateInput {
    #[serde(default)]
    all: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput, Default)]
#[serde(default, deny_unknown_fields)]
struct SkillsRefreshInput {
    /// Include discovery diagnostics in the human-readable response.
    verbose: bool,
}

pub(crate) struct SkillsPlugin {
    workspace_root: OnceLock<PathBuf>,
    host: OnceLock<Arc<dyn HostClient>>,
    config: OnceLock<SkillsPluginConfig>,
    catalog_state: Mutex<CatalogState>,
    watcher: Mutex<Option<SkillCatalogWatcher>>,
    watcher_generation: Arc<AtomicU64>,
    active: Mutex<BTreeMap<i64, ActiveSkill>>,
    trusted_content: Mutex<std::collections::BTreeSet<String>>,
}

fn skills_config_schema() -> serde_json::Value {
    let mut schema = agena_runtime_tools::tool::definition::json_schema_for_default(
        SkillsPluginConfig::default(),
    );
    for (pointer, title, description) in [
        (
            "",
            "Skills Plugin Config",
            "Controls discovery policy for filesystem-backed Skills and slash commands.",
        ),
        (
            "/properties/disabled",
            "Disabled Skills",
            "Canonical Skill or command names to hide from list/get/run; an active persisted Skill with a disabled name is not restored.",
        ),
        (
            "/properties/additional_roots",
            "Additional Skill Roots",
            "Workspace-relative directories scanned after the standard roots. Their contents still require exact-hash approval before activation.",
        ),
        (
            "/properties/additional_command_roots",
            "Additional Command Roots",
            "Workspace-relative directories scanned after the standard command roots.",
        ),
        (
            "/properties/implicit_invocation",
            "Implicit Invocation",
            "Path-gated, trust-preserving automatic Skill activation policy.",
        ),
        (
            "/properties/implicit_invocation/properties/enabled",
            "Enabled",
            "Disable all automatic Skill activation. Skills must separately opt in with allow-implicit-invocation and workspace-relative paths.",
        ),
        (
            "/properties/implicit_invocation/properties/max_candidates",
            "Maximum Candidates",
            "Bounds the number of trusted, path-matched Skills considered for one user prompt.",
        ),
        (
            "/properties/implicit_invocation/properties/max_instruction_chars",
            "Instruction Budget",
            "Maximum character length of automatically injected Skill instructions; this is a tokenizer-neutral context budget.",
        ),
        (
            "/properties/watcher",
            "Filesystem Watcher",
            "Use the platform filesystem watcher to invalidate the Skill catalog after on-disk changes; it never auto-trusts or auto-activates content.",
        ),
        (
            "/properties/watcher/properties/enabled",
            "Enabled",
            "Disable only the OS-level watcher; request-driven discovery remains active for every Skill Tool and hook.",
        ),
    ] {
        agena_runtime_tools::tool::definition::set_schema_metadata(
            &mut schema,
            pointer,
            Some(title),
            Some(description),
        );
    }
    schema
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "skills",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Discover, inspect, and render skills and slash commands.",
    config_schema = skills_config_schema(),
    display = brief
)]
impl SkillsPlugin {
    const TRUST_NAMESPACE: &'static str = "skills.trust.v1";
    const ACTIVE_NAMESPACE: &'static str = "skills.active.v1";
    pub(crate) fn new() -> Self {
        Self {
            workspace_root: OnceLock::new(),
            host: OnceLock::new(),
            config: OnceLock::new(),
            catalog_state: Mutex::new(CatalogState::default()),
            watcher: Mutex::new(None),
            watcher_generation: Arc::new(AtomicU64::new(0)),
            active: Mutex::new(BTreeMap::new()),
            trusted_content: Mutex::new(std::collections::BTreeSet::new()),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: SkillsPluginConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_config(
                ctx.config,
                "invalid skills plugin config",
            )?;
        config.validate_for_workspace(ctx.workspace_root.as_path())?;
        self.workspace_root
            .set(ctx.workspace_root)
            .map_err(|_| PluginError::new("skills plugin initialized more than once"))?;
        self.config
            .set(config)
            .map_err(|_| PluginError::new("skills plugin config initialized more than once"))?;
        self.host
            .set(host)
            .map_err(|_| PluginError::new("skills plugin host initialized more than once"))?;
        self.start_filesystem_watcher()?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    fn render_prompt(body: &str, args: &str) -> String {
        let body = body.trim();
        let args = args.trim();
        if body.contains("$ARGUMENTS") {
            return body.replace("$ARGUMENTS", args);
        }
        if args.is_empty() {
            body.to_string()
        } else {
            format!("{body}\n\nUser arguments:\n{args}")
        }
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::new("skills invoked before init"))
    }

    fn config(&self) -> SkillsPluginConfig {
        self.config.get().cloned().unwrap_or_default()
    }

    fn discovered_catalog(&self) -> SdkResult<DiscoveredCatalog> {
        Ok(self.refresh_catalog()?.catalog)
    }

    fn discovered_tools(&self) -> SdkResult<BTreeMap<String, DiscoveredTool>> {
        Ok(self.discovered_catalog()?.tools)
    }

    #[cfg(test)]
    fn discovered_tools_for_workspace(workspace_root: &Path) -> DiscoveredCatalog {
        Self::discovered_tools_for_workspace_with_config(
            workspace_root,
            &SkillsPluginConfig::default(),
        )
    }

    fn discovered_tools_for_workspace_with_config(
        workspace_root: &Path,
        config: &SkillsPluginConfig,
    ) -> DiscoveredCatalog {
        let (roots, command_roots) = Self::filesystem_roots(workspace_root, config);

        let skill_report = scan_with_diagnostics(&roots);
        let command_report = scan_commands_with_diagnostics(&command_roots);

        let mut tools: BTreeMap<String, DiscoveredTool> = agena_skills::bundled::all()
            .into_iter()
            .map(|skill| {
                let name = skill.frontmatter.name.clone();
                (
                    name,
                    DiscoveredTool {
                        skill,
                        kind: DiscoveredToolKind::Skill,
                        origin: SkillOrigin::Bundled,
                    },
                )
            })
            .collect();
        for (name, tool) in Self::plugin_contributed_tools() {
            tools.insert(name, tool);
        }
        for skill in skill_report.skills {
            let name = skill.frontmatter.name.clone();
            tools.insert(
                name,
                DiscoveredTool {
                    skill,
                    kind: DiscoveredToolKind::Skill,
                    origin: SkillOrigin::Filesystem,
                },
            );
        }
        for command in command_report.skills {
            tools.insert(
                command.frontmatter.name.clone(),
                DiscoveredTool {
                    skill: command,
                    kind: DiscoveredToolKind::Command,
                    origin: SkillOrigin::Filesystem,
                },
            );
        }
        tools.retain(|name, _| !config.disabled_name(name));
        let mut diagnostics = skill_report.diagnostics;
        diagnostics.extend(command_report.diagnostics);
        DiscoveredCatalog { tools, diagnostics }
    }

    fn filesystem_roots(
        workspace_root: &Path,
        config: &SkillsPluginConfig,
    ) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let workspace =
            Some(workspace_root.to_path_buf()).filter(|path| !path.as_os_str().is_empty());
        let mut roots = default_roots(workspace.as_deref());
        let mut command_roots = default_command_roots(workspace.as_deref());
        roots.extend(
            config
                .additional_roots
                .iter()
                .map(|root| workspace_root.join(root)),
        );
        command_roots.extend(
            config
                .additional_command_roots
                .iter()
                .map(|root| workspace_root.join(root)),
        );
        roots.sort();
        roots.dedup();
        command_roots.sort();
        command_roots.dedup();
        (roots, command_roots)
    }

    /// Install a platform watcher over existing catalog roots. For a root
    /// that does not yet exist, watch its nearest existing ancestor without
    /// recursion so creation of `.agena/skills` is still observed without
    /// recursively monitoring an entire workspace. The normal next request
    /// rescans and can then use the newly created root.
    fn start_filesystem_watcher(&self) -> SdkResult<()> {
        if !self.config().watcher.enabled {
            return Ok(());
        }
        let (skill_roots, command_roots) =
            Self::filesystem_roots(self.workspace_root()?, &self.config());
        let mut desired = std::collections::BTreeMap::<PathBuf, RecursiveMode>::new();
        for root in skill_roots.into_iter().chain(command_roots) {
            let (path, mode) = watcher_target(root.as_path());
            desired
                .entry(path)
                .and_modify(|current| {
                    if matches!(mode, RecursiveMode::Recursive) {
                        *current = RecursiveMode::Recursive;
                    }
                })
                .or_insert(mode);
        }

        let generation = Arc::clone(&self.watcher_generation);
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok() {
                    generation.fetch_add(1, Ordering::AcqRel);
                }
            })
            .map_err(|error| {
                PluginError::new(format!("cannot start Skill filesystem watcher: {error}"))
            })?;
        let mut watched_paths = Vec::new();
        for (path, mode) in desired {
            match watcher.watch(path.as_path(), mode) {
                Ok(()) => watched_paths.push(path),
                Err(error) => tracing::warn!(
                    target: "agena_skills::watcher",
                    path = %path.display(),
                    "cannot watch Skill catalog path: {error}"
                ),
            }
        }
        *self
            .watcher
            .lock()
            .map_err(|_| PluginError::new("skills watcher lock poisoned"))? =
            Some(SkillCatalogWatcher {
                _watcher: watcher,
                watched_paths,
            });
        Ok(())
    }

    fn watcher_status(&self) -> SdkResult<(bool, usize, u64)> {
        let watcher = self
            .watcher
            .lock()
            .map_err(|_| PluginError::new("skills watcher lock poisoned"))?;
        Ok((
            self.config().watcher.enabled,
            watcher
                .as_ref()
                .map(|watcher| watcher.watched_paths.len())
                .unwrap_or_default(),
            self.watcher_generation.load(Ordering::Acquire),
        ))
    }

    /// Project declarative Skill contributions from loaded plugin manifests.
    /// Plugin manifests are already authenticated/validated by PluginHost;
    /// their instructions are still treated as non-bundled content and need
    /// the normal exact-hash activation confirmation before use.
    fn plugin_contributed_tools() -> BTreeMap<String, DiscoveredTool> {
        let Some(host) = agena_runtime_plugins::plugin_slot::current_plugin_host() else {
            return BTreeMap::new();
        };
        let mut contributions = host
            .plugins()
            .iter()
            .map(|plugin| {
                (
                    plugin.key().to_string(),
                    plugin.manifest.version.clone(),
                    plugin.manifest.skills.clone(),
                )
            })
            .collect::<Vec<_>>();
        contributions.sort_by(|left, right| left.0.cmp(&right.0));
        Self::plugin_contributed_tools_from_manifests(contributions)
    }

    fn plugin_contributed_tools_from_manifests(
        manifests: impl IntoIterator<
            Item = (
                String,
                String,
                Vec<agena_plugin_host::sdk::PluginSkillDefinition>,
            ),
        >,
    ) -> BTreeMap<String, DiscoveredTool> {
        let mut tools = BTreeMap::new();
        for (plugin_id, plugin_version, skills) in manifests {
            for definition in skills {
                let name = definition.name.clone();
                let skill = Skill {
                    frontmatter: SkillFrontmatter {
                        name: definition.name,
                        description: definition.description,
                        allowed_tools: definition.allowed_tools,
                        model: definition.model,
                        aliases: definition.aliases,
                        user_invocable: definition.user_invocable,
                        allow_implicit_invocation: definition.allow_implicit_invocation,
                        paths: definition.paths,
                        dependencies: SkillDependencies {
                            tools: definition.dependencies.tools,
                            mcp: definition.dependencies.mcp,
                            environment: definition.dependencies.environment,
                        },
                    },
                    body: definition.instructions,
                    source_path: None,
                };
                tools.insert(
                    name,
                    DiscoveredTool {
                        skill,
                        kind: DiscoveredToolKind::Skill,
                        origin: SkillOrigin::Plugin {
                            plugin_id: plugin_id.clone(),
                            plugin_version: plugin_version.clone(),
                        },
                    },
                );
            }
        }
        tools
    }

    fn catalog_fingerprint(catalog: &DiscoveredCatalog) -> String {
        let mut digest = Sha256::new();
        for (name, tool) in &catalog.tools {
            digest.update(name.as_bytes());
            digest.update([0]);
            digest.update(tool.kind.as_ref().as_bytes());
            digest.update([0]);
            digest.update(tool.origin.source_label().as_bytes());
            digest.update([0]);
            digest.update(tool.skill.content_hash().as_bytes());
            digest.update([0]);
            if let Some(path) = tool.skill.source_path.as_ref() {
                digest.update(path.to_string_lossy().as_bytes());
            }
            digest.update([0xff]);
        }
        for diagnostic in &catalog.diagnostics {
            digest.update(diagnostic.path.to_string_lossy().as_bytes());
            digest.update([0]);
            digest.update(diagnostic.error.as_bytes());
            digest.update([0xfe]);
        }
        hex::encode(digest.finalize())
    }

    /// Rescan on demand instead of keeping stale prompt packages in memory.
    /// This gives every skills Tool and active-skill hook a request-driven hot
    /// reload boundary; `skills.refresh` surfaces the generation/delta to the
    /// caller when a user or model needs an explicit audit point.
    fn refresh_catalog(&self) -> SdkResult<CatalogRefresh> {
        let catalog = Self::discovered_tools_for_workspace_with_config(
            self.workspace_root()?,
            &self.config(),
        );
        let fingerprint = Self::catalog_fingerprint(&catalog);
        let mut state = self
            .catalog_state
            .lock()
            .map_err(|_| PluginError::new("skills catalog-state lock poisoned"))?;
        let changed = state.fingerprint.as_deref() != Some(fingerprint.as_str());
        if changed {
            state.generation = state.generation.saturating_add(1);
            state.fingerprint = Some(fingerprint.clone());
        }
        Ok(CatalogRefresh {
            catalog,
            fingerprint,
            generation: state.generation,
            changed,
        })
    }

    fn active_skills(&self) -> SdkResult<std::sync::MutexGuard<'_, BTreeMap<i64, ActiveSkill>>> {
        self.active
            .lock()
            .map_err(|_| PluginError::new("skills active-state lock poisoned"))
    }

    /// Select at most one automatically eligible Skill for a user prompt.
    /// This is intentionally a deterministic local ranking, not an LLM
    /// classifier: a Skill must opt in, declare path glob gates, pass those
    /// gates against a path-looking token in the prompt, and (unless bundled)
    /// already have exact-hash trust.  No untrusted instructions are ever
    /// injected merely because a prompt happens to mention a filename.
    async fn implicit_activation_for_prompt(
        &self,
        session_id: i64,
        prompt: &str,
    ) -> SdkResult<Option<(ActiveSkill, usize)>> {
        let policy = self.config().implicit_invocation.clone();
        if !policy.enabled || self.active_skills()?.contains_key(&session_id) {
            return Ok(None);
        }
        let paths = Self::prompt_path_candidates(prompt);
        if paths.is_empty() {
            return Ok(None);
        }

        let catalog = self.discovered_catalog()?;
        let mut candidates = Vec::new();
        for (name, discovered) in catalog.tools {
            let frontmatter = &discovered.skill.frontmatter;
            if !frontmatter.allow_implicit_invocation || frontmatter.paths.is_empty() {
                continue;
            }
            let rendered = Self::render_prompt(discovered.skill.body.as_str(), "");
            if rendered.chars().count() > policy.max_instruction_chars as usize {
                continue;
            }
            let score = Self::path_gate_score(frontmatter.paths.as_slice(), paths.as_slice());
            if score == 0 {
                continue;
            }
            candidates.push((name, discovered, rendered, score));
        }
        candidates.sort_by(|left, right| right.3.cmp(&left.3).then_with(|| left.0.cmp(&right.0)));
        candidates.truncate(policy.max_candidates as usize);

        for (name, discovered, prompt, score) in candidates {
            if discovered.origin.needs_activation_trust() {
                let content_hash = discovered.skill.content_hash();
                let trusted_in_memory = self
                    .trusted_content
                    .lock()
                    .map_err(|_| PluginError::new("skill trust registry lock poisoned"))?
                    .contains(content_hash.as_str());
                let trusted_persistently = !trusted_in_memory
                    && self
                        .persisted_content_is_trusted(content_hash.as_str())
                        .await;
                if !trusted_in_memory && !trusted_persistently {
                    continue;
                }
            }
            // Dependencies remain an activation invariant.  A missing MCP
            // server or capability simply disqualifies this candidate rather
            // than blocking the user's prompt or selecting a weaker fallback
            // without an explicit declaration.
            if self
                .validate_skill_dependencies(name.as_str(), &discovered)
                .await
                .is_err()
            {
                continue;
            }
            let active = ActiveSkill {
                canonical_name: name,
                prompt,
                allowed_tools: discovered.skill.frontmatter.allowed_tools.clone(),
                // A background path match must not silently change the
                // session's provider/model route. Explicit `skills.run`
                // retains that documented model-selection behavior.
                model: None,
                source_path: discovered.skill.source_path.clone(),
                source: discovered.origin.source_label(),
                content_hash: discovered.skill.content_hash(),
            };
            self.active_skills()?.insert(session_id, active.clone());
            return Ok(Some((active, score)));
        }
        Ok(None)
    }

    fn prompt_path_candidates(prompt: &str) -> Vec<String> {
        let mut paths = prompt
            .split_whitespace()
            .map(|token| {
                token
                    .trim_matches(|character: char| {
                        matches!(
                            character,
                            '`' | '\'' | '"' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ':' | ';'
                        )
                    })
                    .trim_start_matches("./")
                    .replace('\\', "/")
            })
            .filter(|token| {
                !token.is_empty()
                    && !token.starts_with('/')
                    && !token.contains("..")
                    && (token.contains('/') || token.contains('.'))
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        paths
    }

    fn path_gate_score(patterns: &[String], paths: &[String]) -> usize {
        let mut builder = GlobSetBuilder::new();
        let mut valid_pattern_count = 0usize;
        for pattern in patterns {
            let pattern = pattern.trim().trim_start_matches("./");
            if pattern.is_empty()
                || pattern.starts_with('/')
                || pattern.split('/').any(|component| component == "..")
            {
                continue;
            }
            if let Ok(glob) = Glob::new(pattern) {
                builder.add(glob);
                valid_pattern_count += 1;
            }
        }
        if valid_pattern_count == 0 {
            return 0;
        }
        let Ok(set) = builder.build() else {
            return 0;
        };
        paths.iter().filter(|path| set.is_match(path)).count()
    }

    async fn persisted_content_is_trusted(&self, content_hash: &str) -> bool {
        let Some(host) = self.host.get() else {
            return false;
        };
        host.storage_get(HostStorageGetRequest {
            scope: HostStorageScope::Workspace,
            visibility: Default::default(),
            namespace: Self::TRUST_NAMESPACE.to_string(),
            key: content_hash.to_string(),
        })
        .await
        .ok()
        .and_then(|response| response.value)
        .is_some_and(|value| value == "trusted")
    }

    async fn persist_content_trust(&self, content_hash: &str) {
        let Some(host) = self.host.get() else {
            return;
        };
        if let Err(error) = host
            .storage_set(HostStorageSetRequest {
                scope: HostStorageScope::Workspace,
                visibility: Default::default(),
                namespace: Self::TRUST_NAMESPACE.to_string(),
                key: content_hash.to_string(),
                value: "trusted".to_string(),
            })
            .await
        {
            tracing::warn!(
                target: "agena_skills::trust",
                content_hash,
                "failed to persist skill trust; using this-process trust only: {error}"
            );
        }
    }

    /// Resolve a session's active skill from memory or session-private
    /// storage. Persisted activations are accepted only when the currently
    /// discovered package still has the exact same content hash; editing a
    /// `SKILL.md` therefore cannot silently retain an old allowlist/prompt.
    async fn active_for_session(&self, session_id: i64) -> SdkResult<Option<ActiveSkill>> {
        if let Some(active) = self.active_skills()?.get(&session_id).cloned() {
            return Ok(Some(active));
        }
        let Some(host) = self.host.get() else {
            return Ok(None);
        };
        let stored = host
            .storage_get(HostStorageGetRequest {
                scope: HostStorageScope::Session,
                visibility: Default::default(),
                namespace: Self::ACTIVE_NAMESPACE.to_string(),
                key: "active".to_string(),
            })
            .await?
            .value;
        let Some(stored) = stored else {
            return Ok(None);
        };
        let mut active: ActiveSkill = match serde_json::from_str(&stored) {
            Ok(active) => active,
            Err(error) => {
                tracing::warn!(target: "agena_skills", %error, "discarding malformed persisted active skill");
                self.clear_persisted_active().await?;
                return Ok(None);
            }
        };
        if active.source.trim().is_empty() {
            active.source = active
                .source_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "legacy".to_string());
        }
        let current_hash = self
            .discovered_tools()?
            .get(active.canonical_name.as_str())
            .map(|tool| tool.skill.content_hash());
        if current_hash.as_deref() != Some(active.content_hash.as_str()) {
            tracing::warn!(
                target: "agena_skills",
                skill = %active.canonical_name,
                "discarding persisted active skill whose package no longer matches its approved hash"
            );
            self.clear_persisted_active().await?;
            return Ok(None);
        }
        self.active_skills()?.insert(session_id, active.clone());
        Ok(Some(active))
    }

    async fn persist_active(&self, active: &ActiveSkill) -> SdkResult<()> {
        let host = self.host.get().ok_or_else(|| {
            PluginError::new("skills host unavailable before persisting activation")
        })?;
        let value = serde_json::to_string(active)
            .map_err(|error| PluginError::new(format!("serialize active skill: {error}")))?;
        host.storage_set(HostStorageSetRequest {
            scope: HostStorageScope::Session,
            visibility: Default::default(),
            namespace: Self::ACTIVE_NAMESPACE.to_string(),
            key: "active".to_string(),
            value,
        })
        .await
    }

    async fn clear_persisted_active(&self) -> SdkResult<()> {
        let Some(host) = self.host.get() else {
            return Ok(());
        };
        host.storage_delete(HostStorageDeleteRequest {
            scope: HostStorageScope::Session,
            visibility: Default::default(),
            namespace: Self::ACTIVE_NAMESPACE.to_string(),
            key: "active".to_string(),
        })
        .await
    }

    async fn confirm_skill_trust(
        &self,
        name: &str,
        discovered: &DiscoveredTool,
        content_hash: &str,
    ) -> SdkResult<()> {
        const APPROVE: &str = "Trust and activate";
        const REJECT: &str = "Cancel";
        let source = discovered.origin.source_label();
        let body = format!(
            "A non-bundled {} is requesting activation. Its instructions will be injected into the session and its allowed-tool policy will be enforced by the runtime.\n\n- Name: `{name}`\n- Source: `{source}`\n- Content SHA-256: `{content_hash}`\n- Allowed tools: {}\n- Resource paths: {}\n- Tool dependencies: {}\n- MCP dependencies: {}\n- Environment dependencies: {}",
            discovered.kind,
            display_values(&discovered.skill.frontmatter.allowed_tools),
            display_values(&discovered.skill.frontmatter.paths),
            display_values(&discovered.skill.frontmatter.dependencies.tools),
            display_values(&discovered.skill.frontmatter.dependencies.mcp),
            display_values(&discovered.skill.frontmatter.dependencies.environment),
        );
        let response = self
            .host
            .get()
            .ok_or_else(|| PluginError::new("skills host unavailable before trust confirmation"))?
            .ask_user(AskUserRequest {
                title: format!("Trust {name}?"),
                body_markdown: body,
                kind: "skill_trust".to_string(),
                submit_label: "Continue".to_string(),
                cancel_label: REJECT.to_string(),
                auto_resolution_ms: None,
                questions: vec![AskUserQuestion {
                    id: "trust".to_string(),
                    header: "Skill trust".to_string(),
                    question: "Allow this exact content revision to activate for this runtime?"
                        .to_string(),
                    options: vec![
                        AskUserOption {
                            label: APPROVE.to_string(),
                            description:
                                "Trust only this content hash and activate it for the session."
                                    .to_string(),
                            preview_markdown: String::new(),
                        },
                        AskUserOption {
                            label: REJECT.to_string(),
                            description: "Do not activate this skill.".to_string(),
                            preview_markdown: String::new(),
                        },
                    ],
                    multiple: false,
                    allow_custom: false,
                }],
                prompt: String::new(),
                options: Vec::new(),
                allow_free_text: false,
            })
            .await?;
        let decision = response
            .answers
            .get("trust")
            .and_then(|answers| answers.first())
            .map(String::as_str)
            .or_else(|| (!response.reply.trim().is_empty()).then_some(response.reply.trim()));
        if response.cancelled || response.timed_out || decision != Some(APPROVE) {
            return Err(PluginError::new(format!(
                "activation of untrusted skill '{name}' was not approved"
            )));
        }
        Ok(())
    }

    async fn validate_skill_dependencies(
        &self,
        name: &str,
        discovered: &DiscoveredTool,
    ) -> SdkResult<()> {
        let dependencies = &discovered.skill.frontmatter.dependencies;
        if dependencies.tools.is_empty()
            && dependencies.mcp.is_empty()
            && dependencies.environment.is_empty()
        {
            return Ok(());
        }
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::new("skills host unavailable for dependency checks"))?;
        let available_tools = if dependencies.tools.is_empty() {
            Vec::new()
        } else {
            host.list_tools().await?
        };
        let missing_tools = dependencies
            .tools
            .iter()
            .filter(|required| {
                !available_tools.iter().any(|tool| {
                    tool.name.eq_ignore_ascii_case(required.trim())
                        || tool
                            .name
                            .rsplit('.')
                            .next()
                            .is_some_and(|bare| bare.eq_ignore_ascii_case(required.trim()))
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        let missing_mcp = if dependencies.mcp.is_empty() {
            Vec::new()
        } else {
            let servers = host.mcp_list_servers().await?.servers;
            dependencies
                .mcp
                .iter()
                .filter(|required| {
                    !servers
                        .iter()
                        .any(|server| server.eq_ignore_ascii_case(required.trim()))
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        let missing_environment = dependencies
            .environment
            .iter()
            .filter(|name| std::env::var_os(name.trim()).is_none())
            .cloned()
            .collect::<Vec<_>>();
        if missing_tools.is_empty() && missing_mcp.is_empty() && missing_environment.is_empty() {
            return Ok(());
        }
        Err(PluginError::new(format!(
            "skill '{name}' has unresolved dependencies: tools=[{}], mcp=[{}], environment=[{}]",
            missing_tools.join(", "),
            missing_mcp.join(", "),
            missing_environment.join(", "),
        )))
    }

    fn tool_is_allowed(active: &ActiveSkill, input: &ToolBeforeInput) -> bool {
        if active.allowed_tools.is_empty() {
            return true;
        }
        let full = input.tool.to_string();
        let plugin = input.plugin_key().to_string();
        let compact_plugin = plugin.strip_prefix("agena.").unwrap_or(plugin.as_str());
        let compact = format!("{compact_plugin}.{}", input.tool_name());
        let bare = input.tool_name();
        if plugin == "agena.skills" || plugin == "agena.tools" {
            return true;
        }
        active.allowed_tools.iter().any(|allowed| {
            let allowed = allowed.trim();
            allowed.eq_ignore_ascii_case(full.as_str())
                || allowed.eq_ignore_ascii_case(compact.as_str())
                || allowed.eq_ignore_ascii_case(bare)
        })
    }

    fn normalize_kind_filter(kind: Option<&str>) -> SdkResult<Option<DiscoveredToolKind>> {
        match kind.map(str::trim).filter(|value| !value.is_empty()) {
            None => Ok(None),
            Some("skill") => Ok(Some(DiscoveredToolKind::Skill)),
            Some("command") => Ok(Some(DiscoveredToolKind::Command)),
            Some(other) => Err(PluginError::invalid_params(format!(
                "unknown kind '{other}', expected 'skill' or 'command'"
            ))),
        }
    }

    fn paginate<T>(
        items: Vec<T>,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> (Vec<T>, usize, usize) {
        let total = items.len();
        let offset = offset.unwrap_or(0) as usize;
        if offset >= total {
            return (Vec::new(), total, offset);
        }
        let limit = limit
            .map(|value| value as usize)
            .unwrap_or(total.saturating_sub(offset));
        let end = offset.saturating_add(limit).min(total);
        let page = items
            .into_iter()
            .skip(offset)
            .take(end.saturating_sub(offset))
            .collect::<Vec<_>>();
        (page, total, offset)
    }

    fn tool_description(discovered_tool: &DiscoveredTool) -> String {
        let category = discovered_tool.kind.as_ref();
        if discovered_tool
            .skill
            .frontmatter
            .description
            .trim()
            .is_empty()
        {
            format!(
                "Generate the '{}' {category} prompt.",
                discovered_tool.skill.frontmatter.name
            )
        } else {
            discovered_tool.skill.frontmatter.description.clone()
        }
    }

    fn resolve_tool<'a>(
        tools: &'a BTreeMap<String, DiscoveredTool>,
        requested: &str,
    ) -> SdkResult<(&'a str, &'a DiscoveredTool)> {
        if let Some((name, tool)) = tools.get_key_value(requested) {
            return Ok((name.as_str(), tool));
        }
        let normalized = requested.trim().to_ascii_lowercase();
        tools
            .iter()
            .find(|(_, tool)| tool.skill.matches(normalized.as_str()))
            .map(|(name, tool)| (name.as_str(), tool))
            .ok_or_else(|| PluginError::invalid_params(format!("unknown skill '{}'", requested)))
    }

    #[hook(tool.definition)]
    fn tool_definition_patch(
        &self,
        input: ToolDefinitionInput,
    ) -> SdkResult<Option<ToolDefinitionPatch>> {
        if input.plugin_key().to_string() != SKILLS_PLUGIN_ID {
            return Ok(None);
        }
        let catalog = self.discovered_catalog()?;
        let tools = catalog.tools;
        let skill_count = tools
            .values()
            .filter(|tool| tool.kind == DiscoveredToolKind::Skill)
            .count();
        let command_count = tools
            .values()
            .filter(|tool| tool.kind == DiscoveredToolKind::Command)
            .count();
        let preview = tools
            .iter()
            .take(12)
            .map(|(name, tool)| {
                format!(
                    "- {} [{}]: {}",
                    name,
                    tool.kind,
                    Self::tool_description(tool)
                )
            })
            .collect::<Vec<_>>();
        let mut lines = vec![
            format!("Currently discovered: {skill_count} skill(s), {command_count} command(s)."),
            "Use `agena.skills/list` to page through discovered items.".to_string(),
            "Use `agena.skills/get` to inspect one item in full.".to_string(),
            "Use `agena.skills/run` with `name` and optional `args` to activate one skill for the current session.".to_string(),
            "Use `agena.skills/read_resource` to read a bounded text resource from a skill package.".to_string(),
            "Use `agena.skills/refresh` to rescan filesystem-backed Skills and inspect the catalog generation.".to_string(),
            "Use `agena.skills/status` and `agena.skills/deactivate` to inspect or clear session activation.".to_string(),
        ];
        if !catalog.diagnostics.is_empty() {
            lines.push(format!(
                "Discovery diagnostics: {} invalid or unreadable item(s); call list with verbose=true for details.",
                catalog.diagnostics.len()
            ));
        }
        if !preview.is_empty() {
            lines.push("Preview:".to_string());
            lines.extend(preview);
        }

        let summary = match input.tool_name() {
            "list" => Some(format!(
                "List discovered skills and slash commands. Currently {skill_count} skill(s) and {command_count} command(s)."
            )),
            "get" => Some("Read one discovered skill or slash command.".to_string()),
            "run" => Some(format!(
                "Activate one discovered skill or slash command for this session. Currently {skill_count} skill(s) and {command_count} command(s)."
            )),
            "read_resource" => {
                Some("Read a bounded UTF-8 resource contained by one skill package.".to_string())
            }
            "refresh" => Some(
                "Rescan filesystem-backed Skills and report whether the catalog changed."
                    .to_string(),
            ),
            "status" => Some("Inspect the skill currently activated for this session.".to_string()),
            "deactivate" => {
                Some("Deactivate the skill currently active for this session.".to_string())
            }
            _ => None,
        };

        Ok(Some(ToolDefinitionPatch {
            summary,
            help: Some(lines.join("\n")),
            description_mode: None,
            input_schema: None,
        }))
    }

    #[tool(
        summary = "List discovered skills and slash commands.",
        read_only,
        discovery,
        ui_display = detailed,
        concurrency_safe
    )]
    async fn invoke_list(&self, input: &SkillsListInput) -> SdkResult<ToolInvokeOutput> {
        let catalog = self.discovered_catalog()?;
        let kind_filter = Self::normalize_kind_filter(input.kind.as_deref())?;
        let entries = catalog
            .tools
            .into_iter()
            .filter(|(_, tool)| kind_filter.is_none_or(|kind| tool.kind == kind))
            .collect::<Vec<_>>();
        let (entries, total, offset) = Self::paginate(entries, input.offset, input.limit);
        let mut lines = vec![format!(
            "Discovered skill tool(s): returned {}/{} starting at offset {}.",
            entries.len(),
            total,
            offset
        )];
        for (name, tool) in &entries {
            if input.verbose {
                lines.push(format!(
                    "- {} [{}]: {}{}",
                    name,
                    tool.kind,
                    Self::tool_description(tool),
                    format!(" ({})", tool.origin.source_label())
                ));
            } else {
                lines.push(format!("- {} [{}]", name, tool.kind));
            }
        }
        if input.verbose && !catalog.diagnostics.is_empty() {
            lines.push("Discovery diagnostics:".to_string());
            lines.extend(catalog.diagnostics.iter().map(|diagnostic| {
                format!("- {}: {}", diagnostic.path.display(), diagnostic.error)
            }));
        }
        let payload = serde_json::json!({
            "tools": entries.iter().map(|(name, tool)| serde_json::json!({
                "name": name,
                "kind": tool.kind.to_string(),
                "summary": Self::tool_description(tool),
                "aliases": tool.skill.frontmatter.aliases,
                "allowed_tools": tool.skill.frontmatter.allowed_tools,
                "model": tool.skill.frontmatter.model,
                "user_invocable": tool.skill.frontmatter.user_invocable,
                "allow_implicit_invocation": tool.skill.frontmatter.allow_implicit_invocation,
                "source_path": tool.skill.source_path,
                "source": tool.origin.source_label(),
                "content_hash": tool.skill.content_hash(),
            })).collect::<Vec<_>>(),
            "diagnostics": catalog.diagnostics.iter().map(|diagnostic| serde_json::json!({
                "path": diagnostic.path,
                "error": diagnostic.error,
            })).collect::<Vec<_>>(),
            "total": total,
            "offset": offset,
            "returned": entries.len(),
            "kind": kind_filter.map(|kind| kind.to_string()),
        });
        Ok(ToolInvokeOutput::from_parts(
            "skills list",
            lines.join("\n"),
            Some(payload),
            std::collections::BTreeMap::from([
                ("total_tools".to_string(), total.to_string()),
                ("returned_tools".to_string(), entries.len().to_string()),
                ("offset".to_string(), offset.to_string()),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Read one discovered skill or slash command.",
        read_only,
        discovery,
        ui_display = detailed,
        concurrency_safe
    )]
    async fn invoke_get(&self, input: &SkillsGetInput) -> SdkResult<ToolInvokeOutput> {
        let tools = self.discovered_tools()?;
        let (name, discovered_tool) = Self::resolve_tool(&tools, input.name.as_str())?;
        let summary = Self::tool_description(discovered_tool);
        let body = discovered_tool.skill.body.trim();
        let text = format!(
            "Name: {name}\nKind: {}\nSummary: {}\n\nBody:\n{}",
            discovered_tool.kind, summary, body
        );
        let payload = serde_json::json!({
            "name": name,
            "kind": discovered_tool.kind.to_string(),
            "summary": summary,
            "body": body,
            "aliases": discovered_tool.skill.frontmatter.aliases,
            "allowed_tools": discovered_tool.skill.frontmatter.allowed_tools,
            "model": discovered_tool.skill.frontmatter.model,
            "user_invocable": discovered_tool.skill.frontmatter.user_invocable,
            "allow_implicit_invocation": discovered_tool.skill.frontmatter.allow_implicit_invocation,
            "paths": discovered_tool.skill.frontmatter.paths,
            "dependencies": discovered_tool.skill.frontmatter.dependencies,
            "source_path": discovered_tool.skill.source_path,
            "source": discovered_tool.origin.source_label(),
            "content_hash": discovered_tool.skill.content_hash(),
        });
        Ok(ToolInvokeOutput::from_parts(
            format!("skills get {name}"),
            text,
            Some(payload),
            std::collections::BTreeMap::from([
                ("name".to_string(), name.to_string()),
                ("kind".to_string(), discovered_tool.kind.to_string()),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Activate one discovered skill or slash command for this session.",
        read_only,
        interactive,
        ui_display = detailed,
        capabilities(
            HostCapability::AskUser,
            HostCapability::ListTools,
            HostCapability::McpRegistry,
            HostCapability::SessionRegistry,
            HostCapability::PluginStorage
        )
    )]
    async fn invoke_run(
        &self,
        input: &SkillsRunInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        let tools = self.discovered_tools()?;
        let (name, discovered_tool) = Self::resolve_tool(&tools, input.name.as_str())?;
        if !discovered_tool.skill.frontmatter.user_invocable {
            return Err(PluginError::invalid_params(format!(
                "skill '{name}' does not allow explicit invocation"
            )));
        }
        self.validate_skill_dependencies(name, discovered_tool)
            .await?;
        let content_hash = discovered_tool.skill.content_hash();
        if discovered_tool.origin.needs_activation_trust() {
            let trusted_in_memory = self
                .trusted_content
                .lock()
                .map_err(|_| PluginError::new("skill trust registry lock poisoned"))?
                .contains(content_hash.as_str());
            let trusted_persistently = !trusted_in_memory
                && self
                    .persisted_content_is_trusted(content_hash.as_str())
                    .await;
            if !trusted_in_memory && !trusted_persistently {
                self.confirm_skill_trust(name, discovered_tool, content_hash.as_str())
                    .await?;
                self.persist_content_trust(content_hash.as_str()).await;
            }
            self.trusted_content
                .lock()
                .map_err(|_| PluginError::new("skill trust registry lock poisoned"))?
                .insert(content_hash.clone());
        }
        let prompt = Self::render_prompt(
            discovered_tool.skill.body.as_str(),
            input.args.as_deref().unwrap_or_default(),
        );
        let applied_model = match discovered_tool.skill.frontmatter.model.as_deref() {
            Some(model) => {
                let response = self
                    .host
                    .get()
                    .ok_or_else(|| {
                        PluginError::new("skills host unavailable before model selection")
                    })?
                    .set_session_model(HostSetSessionModelRequest {
                        session_id: Some(context.session_id),
                        model: model.to_string(),
                    })
                    .await?;
                Some(format!("{}/{}", response.provider_id, response.model_id))
            }
            None => None,
        };
        let active = ActiveSkill {
            canonical_name: name.to_string(),
            prompt: prompt.clone(),
            allowed_tools: discovered_tool.skill.frontmatter.allowed_tools.clone(),
            model: discovered_tool.skill.frontmatter.model.clone(),
            source_path: discovered_tool.skill.source_path.clone(),
            source: discovered_tool.origin.source_label(),
            content_hash,
        };
        self.active_skills()?
            .insert(context.session_id, active.clone());
        self.persist_active(&active).await?;
        Ok(ToolInvokeOutput::from_parts(
            format!("activated skill {name}"),
            format!(
                "Activated skill '{name}' for this session. Its prompt will be injected as a typed system context fragment on subsequent model turns.{}{}",
                if active.allowed_tools.is_empty() {
                    String::new()
                } else {
                    format!(
                        " Allowed tools are runtime-enforced: {}.",
                        active.allowed_tools.join(", ")
                    )
                },
                applied_model
                    .as_ref()
                    .map(|model| format!(" Applied session model: {model}."))
                    .unwrap_or_default()
            ),
            Some(serde_json::json!({
                "activation": {
                    "name": name,
                    "kind": discovered_tool.kind.to_string(),
                    "prompt": prompt,
                    "allowed_tools": active.allowed_tools,
                    "model": active.model,
                    "applied_model": applied_model,
                    "source_path": active.source_path,
                    "source": active.source,
                    "content_hash": active.content_hash,
                }
            })),
            std::collections::BTreeMap::from([
                ("workflow".to_string(), name.to_string()),
                (
                    "skill_tool_kind".to_string(),
                    discovered_tool.kind.to_string(),
                ),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Read a bounded UTF-8 resource contained by one skill package.",
        read_only,
        filesystem_read,
        ui_display = detailed,
        concurrency_safe
    )]
    async fn invoke_read_resource(
        &self,
        input: &SkillsReadResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let tools = self.discovered_tools()?;
        let (name, discovered_tool) = Self::resolve_tool(&tools, input.name.as_str())?;
        if !discovered_tool.origin.supports_resources() {
            return Err(PluginError::invalid_params(format!(
                "{} '{}' does not expose filesystem-backed resources",
                discovered_tool.origin.source_label(),
                name
            )));
        }
        let content = discovered_tool
            .skill
            .read_text_resource(Path::new(input.path.as_str()), input.max_bytes as usize)
            .map_err(|error| PluginError::invalid_params(error.to_string()))?;
        Ok(ToolInvokeOutput::from_parts(
            format!("skill resource {name}/{}", input.path),
            content.clone(),
            Some(serde_json::json!({
                "name": name,
                "path": input.path,
                "content": content,
                "bytes": content.len(),
                "content_hash": discovered_tool.skill.content_hash(),
                "source_path": discovered_tool.skill.source_path,
                "source": discovered_tool.origin.source_label(),
            })),
            BTreeMap::from([
                ("skill".to_string(), name.to_string()),
                ("resource_path".to_string(), input.path.clone()),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Rescan filesystem-backed Skills and report the catalog generation.",
        read_only,
        discovery,
        ui_display = detailed,
        concurrency_safe
    )]
    async fn invoke_refresh(&self, input: &SkillsRefreshInput) -> SdkResult<ToolInvokeOutput> {
        let refresh = self.refresh_catalog()?;
        let (watcher_enabled, watched_path_count, watcher_generation) = self.watcher_status()?;
        let skill_count = refresh
            .catalog
            .tools
            .values()
            .filter(|tool| tool.kind == DiscoveredToolKind::Skill)
            .count();
        let command_count = refresh
            .catalog
            .tools
            .values()
            .filter(|tool| tool.kind == DiscoveredToolKind::Command)
            .count();
        let mut lines = vec![format!(
            "Skill catalog generation {} {} ({} skill(s), {} command(s)).",
            refresh.generation,
            if refresh.changed {
                "changed"
            } else {
                "is unchanged"
            },
            skill_count,
            command_count,
        )];
        if input.verbose && !refresh.catalog.diagnostics.is_empty() {
            lines.push("Discovery diagnostics:".to_string());
            lines.extend(refresh.catalog.diagnostics.iter().map(|diagnostic| {
                format!("- {}: {}", diagnostic.path.display(), diagnostic.error)
            }));
        }
        Ok(ToolInvokeOutput::from_parts(
            "skills refresh",
            lines.join("\n"),
            Some(serde_json::json!({
                "changed": refresh.changed,
                "generation": refresh.generation,
                "fingerprint": refresh.fingerprint,
                "skills": skill_count,
                "commands": command_count,
                "watcher": {
                    "enabled": watcher_enabled,
                    "watched_path_count": watched_path_count,
                    "generation": watcher_generation,
                },
                "diagnostics": refresh.catalog.diagnostics.iter().map(|diagnostic| serde_json::json!({
                    "path": diagnostic.path,
                    "error": diagnostic.error,
                })).collect::<Vec<_>>(),
            })),
            BTreeMap::from([
                (
                    "catalog_generation".to_string(),
                    refresh.generation.to_string(),
                ),
                ("catalog_changed".to_string(), refresh.changed.to_string()),
                ("catalog_fingerprint".to_string(), refresh.fingerprint),
                ("watcher_enabled".to_string(), watcher_enabled.to_string()),
                (
                    "watcher_generation".to_string(),
                    watcher_generation.to_string(),
                ),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Inspect the skill currently activated for this session.",
        read_only,
        ui_display = detailed,
        capabilities(HostCapability::PluginStorage),
        concurrency_safe
    )]
    async fn invoke_status(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        let active = self.active_for_session(context.session_id).await?;
        let (watcher_enabled, watched_path_count, watcher_generation) = self.watcher_status()?;
        let Some(active) = active else {
            return Ok(ToolInvokeOutput::from_parts(
                "skill status",
                "No skill is active for this session.",
                Some(serde_json::json!({
                    "active": null,
                    "watcher": {
                        "enabled": watcher_enabled,
                        "watched_path_count": watched_path_count,
                        "generation": watcher_generation,
                    }
                })),
                BTreeMap::new(),
                Vec::new(),
            ));
        };
        Ok(ToolInvokeOutput::from_parts(
            format!("active skill {}", active.canonical_name),
            format!(
                "Active skill: {}. Allowed tools: {}. Model preference: {}.",
                active.canonical_name,
                if active.allowed_tools.is_empty() {
                    "unrestricted".to_string()
                } else {
                    active.allowed_tools.join(", ")
                },
                active.model.as_deref().unwrap_or("inherit")
            ),
            Some(serde_json::json!({
                "active": {
                    "name": active.canonical_name,
                    "allowed_tools": active.allowed_tools,
                    "model": active.model,
                    "source_path": active.source_path,
                    "source": active.source,
                    "content_hash": active.content_hash,
                },
                "watcher": {
                    "enabled": watcher_enabled,
                    "watched_path_count": watched_path_count,
                    "generation": watcher_generation,
                }
            })),
            BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Deactivate the skill currently active for this session.",
        mutating,
        ui_display = detailed,
        capabilities(HostCapability::PluginStorage),
        concurrency_safe
    )]
    async fn invoke_deactivate(
        &self,
        _input: &SkillsDeactivateInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        let removed = self.active_skills()?.remove(&context.session_id);
        self.clear_persisted_active().await?;
        let name = removed
            .as_ref()
            .map(|active| active.canonical_name.as_str());
        Ok(ToolInvokeOutput::from_parts(
            "skill deactivated",
            name.map(|name| format!("Deactivated skill '{name}'."))
                .unwrap_or_else(|| "No skill was active for this session.".to_string()),
            Some(serde_json::json!({ "deactivated": name })),
            BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[hook(prompt.submit)]
    async fn user_prompt_submit(
        &self,
        input: UserPromptSubmitInput,
    ) -> SdkResult<Option<UserPromptSubmitPatch>> {
        let Some((active, score)) = self
            .implicit_activation_for_prompt(input.session_id, input.prompt.as_str())
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(UserPromptSubmitPatch {
            additional_context: Some(format!(
                "Agena automatically activated the path-gated skill '{}' ({} matching path hint{}). Its typed system instructions and runtime-enforced allowed-tool policy now apply for this session. This automatic activation does not change the selected model.",
                active.canonical_name,
                score,
                if score == 1 { "" } else { "s" }
            )),
            ..UserPromptSubmitPatch::default()
        }))
    }

    #[hook(chat.system)]
    async fn chat_system_transform(
        &self,
        input: ChatSystemTransformInput,
    ) -> SdkResult<Option<ChatSystemTransformPatch>> {
        let active = self.active_for_session(input.session_id).await?;
        let Some(active) = active else {
            return Ok(None);
        };
        let metadata = serde_json::json!({
            "name": active.canonical_name,
            "content_hash": active.content_hash,
            "source_path": active.source_path,
            "source": active.source,
            "allowed_tools": active.allowed_tools,
            "model_preference": active.model,
        });
        Ok(Some(ChatSystemTransformPatch {
            append: Some(format!(
                "\n\n<agena_activated_skill>\nMetadata: {}\nInstructions:\n{}\n</agena_activated_skill>",
                serde_json::to_string(&metadata)
                    .map_err(|error| PluginError::new(error.to_string()))?,
                active.prompt
            )),
            ..ChatSystemTransformPatch::default()
        }))
    }

    #[hook(tool.before)]
    async fn tool_execute_before(
        &self,
        input: ToolBeforeInput,
    ) -> SdkResult<Option<ToolBeforePatch>> {
        let active = self.active_for_session(input.session_id).await?;
        let Some(active) = active else {
            return Ok(None);
        };
        if Self::tool_is_allowed(&active, &input) {
            return Ok(None);
        }
        Ok(Some(ToolBeforePatch {
            abort_reason: Some(format!(
                "active skill '{}' restricts tools to [{}]; '{}' is not allowed",
                active.canonical_name,
                active.allowed_tools.join(", "),
                input.tool
            )),
            metadata: BTreeMap::from([
                ("skill".to_string(), active.canonical_name),
                ("skill_content_hash".to_string(), active.content_hash),
            ]),
            ..ToolBeforePatch::default()
        }))
    }

    #[hook(session.end)]
    async fn session_end(&self, input: SessionEndInput) -> SdkResult<()> {
        self.active_skills()?.remove(&input.session_id);
        self.clear_persisted_active().await?;
        Ok(())
    }
}
