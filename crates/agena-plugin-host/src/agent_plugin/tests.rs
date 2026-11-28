use super::*;
use agena_plugin_sdk::{
    Plugin,
    configuration::{AgentPluginBridgeConfig, AgentPluginRuntime},
};
use serde_json::json;
use std::time::Duration;

fn write(root: &Path, name: &str, text: impl AsRef<[u8]>) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn claude_experimental_themes_publish_terminal_colors_from_the_declared_directory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(
        &root,
        CLAUDE_MANIFEST,
        json!({
            "name": "review",
            "experimental": { "themes": "./palettes" }
        })
        .to_string(),
    );
    write(
        &root,
        "themes/old.json",
        r##"{"overrides":{"claude":"#000000"}}"##,
    );
    write(
        &root,
        "palettes/contrast.json",
        json!({
            "name": "Theme fixture",
            "overrides": {
                "claude": "#b9f",
                "success": "rgb(80, 250, 123)",
                "permission": "ansi:cyanBright",
                "error": "ansi256(196)",
                "warning": "ansi256(256)"
            }
        })
        .to_string(),
    );
    let mut package = load_agent_plugin(&root).unwrap();
    assert!(package.diagnostics.is_empty(), "{:?}", package.diagnostics);
    assert!(package.watch_paths.contains(&root.join("palettes")));
    assert!(
        package
            .watch_paths
            .contains(&root.join("palettes/contrast.json"))
    );
    let plugin = AgentPluginAdapter::new(
        &"claude.review-team".parse().unwrap(),
        &mut package,
        &[],
        &BTreeMap::new(),
        false,
    )
    .unwrap()
    .manifest();
    let [palette] = plugin.surface.terminal.themes.as_slice() else {
        panic!(
            "expected one theme, got {:?}",
            plugin.surface.terminal.themes
        )
    };
    assert_eq!(palette.id, "claude.review-team.contrast");
    assert_eq!(palette.display_name, "Theme fixture");
    let colors = &palette.colors;
    assert_eq!(colors.accent.as_ref().unwrap().as_str(), "#bb99ff");
    assert_eq!(colors.success.as_ref().unwrap().as_str(), "#50fa7b");
    assert_eq!(colors.info.as_ref().unwrap().as_str(), "light_cyan");
    assert_eq!(colors.danger.as_ref().unwrap().as_str(), "#ff0000");
    assert!(colors.warning.is_none());
}

#[test]
fn codex_portable_manifest_keeps_standard_components_and_takes_overlay_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(
        &root,
        "plugin.json",
        json!({
            "$schema": AGENT_PLUGIN_MANIFEST_SCHEMA,
            "name": "review",
            "extensions": {
                "com.openai": { "interface": { "displayName": "Current Codex extension" } }
            }
        })
        .to_string(),
    );
    write(
        &root,
        CODEX_MANIFEST,
        json!({
            "name": "review",
            "mcpServers": {
                "review": {
                    "command": "different",
                    "env_vars": ["PATH", {"name": "REMOTE", "source": "remote"}],
                    "startup_timeout_sec": 1.25,
                    "startup_timeout_ms": 8_000,
                    "tool_timeout_sec": 4.5,
                    "enabled_tools": ["read", "blocked"],
                    "disabled_tools": ["blocked"]
                },
                "native-only": { "command": "node" }
            }
        })
        .to_string(),
    );
    write(&root, "skills/SKILL.md", "# Not a package");
    write(&root, "skills/review/SKILL.md", "# Review");
    write(&root, "skills/nested/hidden/SKILL.md", "# Not discovered");
    write(
        &root,
        "mcp.json",
        json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
            "mcpServers": {
                "review": {
                    "type": "stdio",
                    "command": "node",
                    "args": ["${PLUGIN_ROOT}/server.js", "${REMAINS_LITERAL}"]
                },
                "bad": { "type": "stdio", "command": "node", "unknownField": true },
                "bad-header": {
                    "type": "streamable-http", "url": "https://example.test/mcp",
                    "headers": {"Accept": "text/plain\ninvalid"}
                }
            }
        })
        .to_string(),
    );

    let package = load_agent_plugin(&root).unwrap();
    assert_eq!(package.format, AgentPluginFormat::AgentPluginsV1);
    assert_eq!(
        package.display_name.as_deref(),
        Some("Current Codex extension")
    );
    assert_eq!(package.skill_dirs, [root.join("skills/review")]);
    assert_eq!(package.mcp_servers.len(), 1);
    let server = &package.mcp_servers[0];
    assert_eq!(
        server.settings.startup_timeout,
        Some(Duration::from_millis(1250))
    );
    assert_eq!(
        server.settings.tool_timeout,
        Some(Duration::from_millis(4500))
    );
    assert_eq!(
        server.settings.enabled_tools,
        Some(BTreeSet::from(["read".into(), "blocked".into()]))
    );
    assert_eq!(
        server.settings.disabled_tools,
        BTreeSet::from(["blocked".into()])
    );
    let mut context = AgentPluginContext {
        root: &root,
        data: &root,
        workspace: &root,
        environment: &[],
        user_config_env: &BTreeMap::new(),
    };
    assert!(server.resolve(&context).err().unwrap().contains("PATH"));
    let environment = ["PATH".to_owned()];
    context.environment = &environment;
    let ResolvedAgentPluginMcp::Stdio {
        command,
        args,
        env,
        cwd,
        ..
    } = server.resolve(&context).unwrap()
    else {
        panic!("expected the portable stdio definition");
    };
    assert_eq!(command, "node");
    assert_eq!(
        args,
        [
            format!("{}/server.js", root.display()),
            "${REMAINS_LITERAL}".into()
        ]
    );
    assert!(!env.contains_key("REMOTE"));
    assert_eq!(cwd, root);
    assert_eq!(package.diagnostics.len(), 2);
}

#[test]
fn claude_lsp_merges_sources_and_resolves_each_callers_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(&root, ".lsp.json", json!({
        "language": { "command": "default-language-server", "extensionToLanguage": {".x": "old"} },
        "broken": { "command": "bad", "extensionToLanguage": {} },
    }).to_string());
    write(
        &root,
        "config/lsp.json",
        json!({
            "language": { "command": "from-file", "extensionToLanguage": {".x": "file"} },
        })
        .to_string(),
    );
    write(
        &root,
        CLAUDE_MANIFEST,
        json!({
            "name": "review",
            "userConfig": {"level": {"default": "warn"}},
            "lspServers": [
                "./config/lsp.json",
                {
                    "language": {
                        "command": "node",
                        "args": ["${CLAUDE_PLUGIN_ROOT}/server.js", "--stdio"],
                        "extensionToLanguage": {".X": "review-lang"},
                        "workspaceFolder": "${CLAUDE_PROJECT_DIR}/src",
                        "env": {"LEVEL": "${user_config.level}"},
                        "initializationOptions": {"index": true},
                        "settings": {"language": {"format": true}},
                        "requestTimeout": 42000
                    }
                }
            ],
            "experimental": {"monitors": [
                {"name": "indexer", "command": "run-indexer", "description": "Index events", "when": "on-skill-invoke:search"}
            ]}
        })
        .to_string(),
    );
    let package = load_agent_plugin(&root).unwrap();
    assert_eq!(package.lsp_servers.len(), 1);
    assert!(package.watch_paths.contains(&root.join("config/lsp.json")));
    assert!(
        package
            .diagnostics
            .iter()
            .any(|message| message.contains("broken"))
    );
    assert_eq!(package.diagnostics.len(), 1);
    assert_eq!(package.monitors.len(), 1);
    let server = &package.lsp_servers[0];
    assert_eq!(server.language_ids["x"], "review-lang");
    assert_eq!(server.request_timeout, Some(42000));
    assert_eq!(
        server.settings.as_ref().unwrap()["language"]["format"],
        true
    );
    for workspace in [root.join("first"), root.join("second")] {
        let resolved = server
            .resolve(&AgentPluginContext {
                root: &root,
                data: &root.join("data"),
                workspace: &workspace,
                environment: &[],
                user_config_env: &BTreeMap::new(),
            })
            .unwrap();
        assert_eq!(resolved.command, "node");
        assert_eq!(
            resolved.args,
            [format!("{}/server.js", root.display()), "--stdio".into()]
        );
        assert_eq!(resolved.env["LEVEL"], "warn");
        assert_eq!(
            resolved.env["CLAUDE_PROJECT_DIR"],
            workspace.to_str().unwrap()
        );
        assert_eq!(resolved.workspace, workspace.join("src"));
    }
}

#[test]
fn claude_merges_current_sources_and_maps_project_and_user_options() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(
        &root,
        CLAUDE_MANIFEST,
        json!({
            "name": "review",
            "displayName": "Review",
            "skills": ["./extra"],
            "userConfig": { "tenant": { "type": "string", "default": "team" } },
            "mcpServers": [
                "./config/servers.json",
                { "review": {
                    "command": "${CLAUDE_PLUGIN_ROOT}/bin/server",
                    "args": ["${CLAUDE_PROJECT_DIR}", "${user_config.tenant}", "${CLAUDE_CONFIG_DIR}"],
                    "cwd": "${CLAUDE_PLUGIN_DATA}/runtime",
                    "timeout": 2400
                }}
            ]
        })
        .to_string(),
    );
    write(&root, "skills/default/SKILL.md", "# Default skill");
    write(&root, "extra/SKILL.md", "# Additional skill");
    write(&root, "bin/server", "server fixture");
    write(
        &root,
        ".mcp.json",
        r#"{"review":{"command":"old"},"default":{"command":"node","timeout":500}}"#,
    );
    write(
        &root,
        "config/servers.json",
        r#"{"servers":{"command":"node"}}"#,
    );

    let package = load_agent_plugin(&root).unwrap();
    assert_eq!(
        package.skill_dirs,
        [root.join("extra"), root.join("skills/default")]
    );
    assert_eq!(
        package
            .mcp_servers
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["default", "review", "servers"]
    );
    assert_eq!(package.mcp_servers[0].settings.tool_timeout, None);
    assert_eq!(
        package.mcp_servers[1].settings.tool_timeout,
        Some(Duration::from_millis(2400))
    );
    let workspace = root.join("workspace");
    let data = root.join("data");
    let context = AgentPluginContext {
        root: &root,
        data: &data,
        workspace: &workspace,
        environment: &[],
        user_config_env: &BTreeMap::new(),
    };
    let ResolvedAgentPluginMcp::Stdio {
        command,
        args,
        env,
        cwd,
        ..
    } = package.mcp_servers[1].resolve(&context).unwrap()
    else {
        panic!("expected the last Claude definition to win");
    };
    assert_eq!(command, root.join("bin/server").to_str().unwrap());
    assert_eq!(
        args,
        [
            workspace.to_string_lossy().into_owned(),
            "team".into(),
            data.to_string_lossy().into_owned(),
        ]
    );
    assert_eq!(env["CLAUDE_PLUGIN_DATA"], data.to_str().unwrap());
    assert_eq!(env["CLAUDE_CONFIG_DIR"], env["CLAUDE_PLUGIN_DATA"]);
    assert_eq!(cwd, data.join("runtime"));
    assert!(cwd.is_dir());
}

#[test]
fn claude_mcp_header_helper_preserves_package_scope_and_redacts_url_templates() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(
        &root,
        CLAUDE_MANIFEST,
        r#"{"name":"review","userConfig":{"tenant":{"type":"string","default":"fixture"}}}"#,
    );
    write(
        &root,
        ".mcp.json",
        r#"{"mcpServers":{
        "remote":{"type":"ws","url":"wss://example.test/${user_config.tenant}/mcp",
                  "headersHelper":"\"$CLAUDE_PLUGIN_ROOT/bin/headers\""},
        "invalid":{"type":"http","url":"https://example.test/mcp",
                   "headersHelper":"echo ${user_config.tenant}"}
    }}"#,
    );
    let package = load_agent_plugin(&root).unwrap();
    assert_eq!(package.mcp_servers.len(), 1);
    assert!(
        package
            .diagnostics
            .iter()
            .any(|message| message.contains("invalid headersHelper"))
    );
    let context = AgentPluginContext {
        root: &root,
        data: &root,
        workspace: &root,
        environment: &[],
        user_config_env: &BTreeMap::new(),
    };
    let ResolvedAgentPluginMcp::Http {
        url,
        headers_helper: Some(helper),
        ..
    } = package.mcp_servers[0].resolve(&context).unwrap()
    else {
        panic!("expected a remote MCP server with a header helper");
    };
    assert_eq!(url, "https://example.test/fixture/mcp");
    assert_eq!(helper.cwd, root);
    assert_eq!(helper.server_name, "remote");
    assert_eq!(helper.display_url, "wss://example.test/REDACTED/mcp");
    assert_eq!(helper.command, "\"$CLAUDE_PLUGIN_ROOT/bin/headers\"");
}

#[test]
fn deepseek_bridge_requires_a_selected_bundle_and_accepts_ordered_patches() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().canonicalize().unwrap();
    let profile = home.join("profiles/headless");
    let root = profile.join("node_modules/@vendor/review");
    write(
        &root,
        "package.json",
        r#"{
        "name":"@vendor/review",
        "version":"1.0.0",
        "dsh":{"bundle":{"patch":["./first.yml","patches/second.yml"]}}
    }"#,
    );
    write(
        &root,
        "first.yml",
        r#"
- insert:
    - id: demo
      name: '@deepseek-ai/dsh-mcp-client'
      config:
        serverName: original
        transport: streamable-http
        url: http://127.0.0.1:3000/mcp
    - id: turned-off
      name: '@deepseek-ai/dsh-mcp-client'
      config:
        serverName: ignored
        transport: stdio
        command: node
    - id: unmapped
      name: '@vendor/dynamic-plugin'
"#,
    );
    write(
        &root,
        "patches/second.yml",
        r#"
- id: demo
  config:
    serverName: updated
    transport: streamable-http
    url: http://127.0.0.1:3001/mcp
- id: turned-off
  disabled: true
"#,
    );
    let mut package = load_agent_plugin(&root).unwrap();
    assert_eq!(package.format, AgentPluginFormat::DeepSeekHarness);
    assert_eq!(package.mcp_servers.len(), 1);
    assert_eq!(package.mcp_servers[0].name, "updated");
    assert_eq!(
        package
            .diagnostics
            .iter()
            .filter(|message| message.contains("@vendor/dynamic-plugin"))
            .count(),
        1
    );
    let config = AgentPluginBridgeConfig {
        runtime: AgentPluginRuntime::DeepseekHarness {
            profile: "headless".into(),
            bundle: Some("@vendor/review".into()),
        },
        config_dir: home,
        executable: None,
        follow_installation: true,
    };
    let key = "vendor.review".parse().unwrap();
    write(
        &profile,
        "package.json",
        r#"{"dsh":{"profile":{"bundles":[]}}}"#,
    );
    assert!(native_installation_path(&config).is_err());
    write(
        &profile,
        "package.json",
        r#"{"dsh":{"profile":{"bundles":["@vendor/review"]}}}"#,
    );
    assert_eq!(native_installation_path(&config).unwrap(), package.root);
    let adapter =
        AgentPluginAdapter::new(&key, &mut package, &[], &BTreeMap::new(), false).unwrap();
    assert_eq!(adapter.manifest().tools[0].name, "run");
}

#[test]
fn deepseek_composes_nested_groups_and_loads_packaged_skill_roots() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(
        &root,
        "package.json",
        r#"{
      "name":"@fixture/bundle",
      "dsh":{"bundle":{"patch":["first.yml","second.yml"]}}
    }"#,
    );
    write(
        &root,
        "first.yml",
        r#"
- insert:
  - id: group
    group: true
    config:
      - id: skills
        name: '@deepseek-ai/dsh-skill-filesystem'
        config:
          bundledSkillDir: old
      - id: unavailable
        name: '@vendor/dynamic-plugin'
        config:
          directory: !!js dshHomePath('private')
  - id: disabled-group
    group: true
    disabled: !!js "true"
    config:
      - name: '@deepseek-ai/dsh-mcp-client'
        config:
          serverName: ignored
          transport: stdio
          command: node
"#,
    );
    write(
        &root,
        "second.yml",
        r#"
- id: group
  insert:
    - id: inside
      name: '@deepseek-ai/dsh-mcp-client'
      config:
        serverName: inside
        transport: streamable-http
        url: !!js "'https://example.test/' + 'mcp'"
- id: skills
  config:
    customSkillDirs: ["bundled/skills"]
"#,
    );
    write(&root, "old/ignored/SKILL.md", "# Ignored");
    write(&root, "bundled/skills/demo/SKILL.md", "# Demo");
    let package = load_agent_plugin(&root).unwrap();
    assert_eq!(package.skill_dirs, [root.join("bundled/skills/demo")]);
    assert_eq!(package.mcp_servers.len(), 1);
    assert_eq!(package.mcp_servers[0].name, "inside");
    assert!(
        package
            .diagnostics
            .iter()
            .any(|message| message.contains("@vendor/dynamic-plugin"))
    );
    assert!(
        !package
            .diagnostics
            .iter()
            .any(|message| message.contains("dshHomePath"))
    );
}

#[tokio::test]
async fn deepseek_runs_packaged_command_exports_and_resolves_base_skill_paths() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(
        &root,
        "package.json",
        r#"{"name":"@example/cmds","exports":{"./commands":{"import":"./src/commands.mjs"}},
        "dsh":{"bundle":{"patch":"./patch/cordis.yml"}}}"#,
    );
    write(
        &root,
        "patch/cordis.yml",
        r#"
- insert:
    - id: review
      name: '@example/cmds/commands'
      config: {prefix: 'Review '}
    - id: local
      name: './local.mjs'
      config: {prefix: 'Check '}
    - id: disabled
      name: './missing.mjs'
      disabled: !!js 'true'
- id: skill-filesystem
  disabled: false
  config:
    customSkillDirs:
      - !!js "process.getBuiltinModule('node:url').fileURLToPath(new URL('bundled skills/', baseUrl))"
"#,
    );
    let module = r#"
      import { textMessage } from '../shared.mjs';
      export const inject = ['commands', 'mcpManager'];
      export function apply(ctx, config) {
        if (typeof process !== 'undefined' || typeof fetch !== 'undefined')
          throw new Error('unexpected host capability');
        ctx.mcpManager.register(config.prefix === 'Review ' ? 'review-server' : 'local-server', {
          transport: 'streamable-http', url: 'https://example.test/mcp',
        });
        ctx.commands.register({
          name: config.prefix === 'Review ' ? 'review' : 'local-check',
          description: 'Review a change', input: {hint: '<target>'},
          async handler(invocation) {
            if (!invocation.rawInput) return {kind: 'error', text: 'Supply a target'};
            invocation.agent.followup(textMessage(config.prefix + invocation.rawInput));
            return {kind: 'success', text: 'Reviewed'};
          },
        });
      }
    "#;
    write(
        &root,
        "shared.mjs",
        "export const textMessage = text => ({role: 'user', source: {kind: 'user'},\n\
         content: [{type: 'text', text}]});",
    );
    write(&root, "src/commands.mjs", module);
    write(&root, "patch/local.mjs", module);
    write(&root, "bundled skills/check/SKILL.md", "# Check");
    let mut package = load_agent_plugin(&root).unwrap();
    assert!(package.diagnostics.is_empty(), "{:?}", package.diagnostics);
    assert_eq!(package.skill_dirs, [root.join("bundled skills/check")]);
    assert!(package.watch_paths.contains(&root.join("src/commands.mjs")));
    assert!(package.watch_paths.contains(&root.join("patch/local.mjs")));
    assert!(package.watch_paths.contains(&root.join("shared.mjs")));
    assert_eq!(
        package
            .mcp_servers
            .iter()
            .map(|server| server.name.as_str())
            .collect::<Vec<_>>(),
        ["local-server", "review-server"]
    );
    let plugin = AgentPluginAdapter::new(
        &"deepseek.cmds".parse().unwrap(),
        &mut package,
        &[],
        &BTreeMap::new(),
        false,
    )
    .unwrap();
    let manifest = plugin.manifest();
    assert_eq!(manifest.commands.len(), 2);
    assert_eq!(
        manifest.commands[0].slash.as_deref(),
        Some("/deepseek.cmds.review")
    );
    use agena_plugin_sdk::{CommandHostEffect, CommandInvokeInput, CommandStatus};
    let mut input = CommandInvokeInput {
        command_id: "review".into(),
        input: json!({"args":"target"}),
        session_id: Some(1),
        call_id: None,
        workspace_root: None,
        slash: None,
        raw: String::new(),
        prompt_context: "hook context".into(),
        cordis_attachments: None,
    };
    let result = plugin.command_invoke(input.clone()).await.unwrap();
    assert_eq!(result.status, CommandStatus::Succeeded);
    assert_eq!(
        result.effects,
        [CommandHostEffect::InsertPrompt {
            prompt: "Review target\n\nhook context".into()
        }]
    );
    input.input = json!({});
    let result = plugin.command_invoke(input.clone()).await.unwrap();
    assert_eq!(result.status, CommandStatus::Failed);
    assert_eq!(result.summary, "Supply a target");
    assert!(result.effects.is_empty());
    input.session_id = None;
    let result = plugin.command_invoke(input).await.unwrap();
    assert_eq!(result.status, CommandStatus::Unavailable);
}

#[test]
fn deepseek_mcp_manager_maps_patched_servers_and_bounded_module_registrations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        "package.json",
        r#"{"name":"@example/mcp","dsh":{"bundle":{"patch":["base.yml","override.yml"]}}}"#,
    );
    write(
        root,
        "base.yml",
        r#"
- id: mcp-manager
  config:
    servers:
      replaced: {transport: stdio, command: node}
- insert:
  - name: './register.mjs'
    id: registrations
  - name: '@deepseek-ai/dsh-mcp-client'
    config:
      serverName: direct
      transport: stdio
      command: node
      toolLoading: on-demand
      catalogDescriptionMaxLength: 40
      toolCallTimeoutMs: 2100
"#,
    );
    write(
        root,
        "override.yml",
        r#"
- id: mcp-manager
  config:
    servers:
      declared:
        transport: streamable-http
        url: https://example.test/declared
        headers: {Authorization: 'Bearer ${credential:FIXTURE_TOKEN}'}
        command: ''
        cwd: ''
        args: []
        env: {}
        startupTimeoutMs: !!js '1000 + 500'
        toolCallTimeoutMs: 2250
        closeTimeoutMs: 3200
      inactive: {enabled: false, transport: stdio, command: node}
      malformed: {transport: stdio, command: node, url: 'https://example.test/wrong'}
"#,
    );
    write(
        root,
        "register.mjs",
        r#"
export const inject = ['mcpManager'];
export function apply(ctx) {
  const create = name => ({transport: 'streamable-http', url: 'https://example.test/' + name});
  ctx.mcpManager.register('registered', create('registered'));
  ctx.mcpManager.register('declared', create('would-replace'));
  ctx.mcpManager.register('inactive', create('would-enable'));
  ctx.mcpManager.register('removed', create('removed'))();
}
"#,
    );
    let package = load_agent_plugin(root).unwrap();
    let [declared, direct, registered] = package.mcp_servers.as_slice() else {
        panic!(
            "expected three active servers, diagnostics {:?}",
            package.diagnostics
        );
    };
    let managed: Vec<_> = package
        .deepseek_managed_mcp_servers
        .iter()
        .map(|entry| (entry.name.as_str(), entry.enabled, entry.server.is_some()))
        .collect();
    assert_eq!(
        managed,
        [
            ("declared", true, true),
            ("inactive", false, true),
            ("malformed", true, false),
            ("registered", true, true),
        ]
    );
    assert_eq!(
        [
            declared.name.as_str(),
            direct.name.as_str(),
            registered.name.as_str()
        ],
        ["declared", "direct", "registered"]
    );
    assert_eq!(
        declared.settings.startup_timeout,
        Some(Duration::from_millis(1500))
    );
    assert_eq!(
        declared.settings.tool_timeout,
        Some(Duration::from_millis(2250))
    );
    assert_eq!(
        declared.settings.close_timeout,
        Some(Duration::from_millis(3200))
    );
    assert_eq!(
        direct.settings.tool_timeout,
        Some(Duration::from_millis(2100))
    );
    assert!(direct.is_stdio());
    assert_eq!(direct.settings.on_demand_description_limit, Some(40));
    assert_eq!(
        registered.settings.startup_timeout,
        Some(Duration::from_secs(30))
    );
    assert_eq!(
        registered.settings.close_timeout,
        Some(Duration::from_secs(5))
    );
    assert!(
        package
            .diagnostics
            .iter()
            .any(|message| message.contains("malformed") && message.contains("unused"))
    );
    assert!(
        package
            .diagnostics
            .iter()
            .any(|message| message.contains("declared") && message.contains("already declared"))
    );
    assert!(
        package
            .diagnostics
            .iter()
            .any(|message| message.contains("inactive") && message.contains("already declared"))
    );
    let context = AgentPluginContext {
        root: &package.root,
        data: &package.root,
        workspace: &package.root,
        environment: &[],
        user_config_env: &BTreeMap::new(),
    };
    let error = declared.resolve(&context).err().unwrap();
    assert!(
        error.contains("FIXTURE_TOKEN") && error.contains("enabled"),
        "{error}"
    );
    let ResolvedAgentPluginMcp::Http { url, .. } = registered.resolve(&context).unwrap() else {
        panic!("expected an HTTP server");
    };
    assert_eq!(url, "https://example.test/registered");
}

#[test]
fn deepseek_mcp_credential_templates_require_enabled_environment_and_static_origin() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        "package.json",
        r#"{"name":"fixtures","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    );
    write(
        root,
        "bundle.yml",
        r#"
- insert:
  - name: '@deepseek-ai/dsh-mcp-client'
    config:
      serverName: authorized
      transport: streamable-http
      url: 'https://example.test/mcp?q=${credential:FIXTURE_TOKEN}'
      expandTemplates: true
      headers:
        Authorization: 'Bearer ${credential:FIXTURE_TOKEN}'
  - name: '@deepseek-ai/dsh-mcp-client'
    config:
      serverName: no-credentials
      transport: stdio
      command: node
      cwd: ''
      expandTemplates: true
  - name: '@deepseek-ai/dsh-mcp-client'
    config:
      serverName: dynamic-origin
      transport: streamable-http
      url: 'https://${credential:FIXTURE_TOKEN}/mcp'
      expandTemplates: true
  - name: '@deepseek-ai/dsh-mcp-client'
    config:
      serverName: invalid
      transport: streamable-http
      url: 'https://example.test/mcp'
      expandTemplates: true
      headers:
        Authorization: 'Bearer ${FIXTURE_TOKEN}'
"#,
    );
    let package = load_agent_plugin(root).unwrap();
    assert_eq!(
        package
            .mcp_servers
            .iter()
            .map(|server| server.name.as_str())
            .collect::<Vec<_>>(),
        ["authorized", "no-credentials"]
    );
    assert!(
        package
            .diagnostics
            .iter()
            .any(|message| message.contains("static endpoint origin"))
    );
    assert!(
        package
            .diagnostics
            .iter()
            .any(|message| message.contains("expected a DeepSeek MCP credential template"))
    );
    let context = AgentPluginContext {
        root: &package.root,
        data: &package.root,
        workspace: &package.root,
        environment: &[],
        user_config_env: &BTreeMap::new(),
    };
    let error = package.mcp_servers[0].resolve(&context).err().unwrap();
    assert!(
        error.contains("FIXTURE_TOKEN") && error.contains("enabled"),
        "{error}"
    );
    let ResolvedAgentPluginMcp::Stdio {
        cwd,
        credential_values,
        ..
    } = package.mcp_servers[1].resolve(&context).unwrap()
    else {
        panic!("expected stdio transport");
    };
    assert_eq!(cwd, package.root);
    assert!(credential_values.is_empty());
}

#[test]
fn deepseek_does_not_inherit_environment_into_javascript_expressions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        "package.json",
        r#"{
        "name":"review",
        "dsh":{"bundle":{"patch":"./cordis.patch.yml"}}
    }"#,
    );
    write(
        root,
        "cordis.patch.yml",
        r#"
- insert:
    - id: dynamic
      name: '@deepseek-ai/dsh-mcp-client'
      config:
        serverName: dynamic
        transport: streamable-http
        url: !!js process.env.EXAMPLE_MCP_URL
"#,
    );
    let package = load_agent_plugin(root).unwrap();
    assert!(package.mcp_servers.is_empty());
    assert!(package.diagnostics[0].contains("did not return a JSON value"));
}

#[test]
fn modern_hook_sources_follow_their_manifest_merge_rules() {
    use agena_plugin_sdk::{HookSubscription, Plugin};

    let claude = tempfile::tempdir().unwrap();
    let root = claude.path();
    write(
        root,
        "hooks/hooks.json",
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo ready"}]}]}}"#,
    );
    write(
        root,
        "extra.json",
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash, Write","hooks":[{"type":"command","command":"node","args":["${CLAUDE_PLUGIN_ROOT}/check.js"],"onFailure":"block"}]}]}}"#,
    );
    write(
        root,
        CLAUDE_MANIFEST,
        json!({
            "name": "review",
            "hooks": [
            "./hooks/hooks.json",
                "./extra.json",
                {"Stop":[{"hooks":[{"type":"http","url":"https://hooks.example.test/stop"}]}]},
                {"UserPromptSubmit":[{"hooks":[{"type":"prompt","prompt":"review"}]}]}
            ],
            "agents": ["./agents/reviewer.md"],
            "outputStyles": "./output-styles",
        })
        .to_string(),
    );
    write(
        root,
        "agents/reviewer.md",
        "---\nname: reviewer\ndescription: Review changes\nmaxTurns: 8\n---\nInspect the proposed change.",
    );
    write(
        root,
        "output-styles/short.md",
        "---\nname: short\ndescription: Short replies\n---\nKeep replies brief.",
    );

    let key = "vendor.review".parse().unwrap();
    let mut package = load_agent_plugin(root).unwrap();
    assert_eq!(package.roles.len(), 2);
    assert_eq!(package.roles[0].max_turns, Some(8));
    assert!(package.roles[1].output_style);
    assert!(
        package
            .watch_paths
            .contains(&package.root.join("extra.json"))
    );
    assert!(package.diagnostics.is_empty(), "{:?}", package.diagnostics);
    let hooks = |package: &mut AgentPluginDescriptor, enable| {
        AgentPluginAdapter::new(&key, package, &[], &BTreeMap::new(), enable)
            .unwrap()
            .manifest()
    };
    assert!(hooks(&mut package, false).hook_handlers.is_empty());
    let manifest = hooks(&mut package, true);
    assert_eq!(manifest.hook_handlers.len(), 4);
    assert!(manifest.hooks.contains(
        HookSubscription::SESSION_START
            | HookSubscription::TOOL_BEFORE
            | HookSubscription::AGENT_STOP
            | HookSubscription::USER_PROMPT_SUBMIT
    ));

    let codex = tempfile::tempdir().unwrap();
    let root = codex.path();
    write(
        root,
        "hooks/hooks.json",
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo default"}]}]}}"#,
    );
    write(root, "plugin.json", json!({
        "$schema": AGENT_PLUGIN_MANIFEST_SCHEMA,
        "name": "review",
        "extensions": {"com.openai": {"hooks": [
            {"hooks": {"PreCompact":[{"matcher":"^auto$","hooks":[{"type":"command","command":"echo compact"}]}]}},
            {"hooks": {"PostModelRequest":[{"hooks":[{"type":"command","command":"echo model"}]}]}}
        ]}}
    }).to_string());
    write(
        root,
        CODEX_MANIFEST,
        r#"{"name":"review","hooks":{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo overlay"}]}]}}}"#,
    );
    let mut package = load_agent_plugin(root).unwrap();
    let manifest = hooks(&mut package, true);
    assert_eq!(manifest.hook_handlers.len(), 2);
    assert_eq!(
        manifest.hooks,
        HookSubscription::INIT
            | HookSubscription::SHUTDOWN
            | HookSubscription::COMPACTION_BEFORE
            | HookSubscription::IMPORTED_MODEL_ATTEMPT_AFTER
    );
}

#[test]
fn codex_imports_mcp_tool_and_background_hooks() {
    use agena_plugin_sdk::HookSubscription;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        CODEX_MANIFEST,
        json!({
            "name": "review",
            "hooks": {
                "UserPromptSubmit": [{"hooks": [
                    {"type":"mcp_tool","server":"gate","tool":"check","input":{"value":"${prompt}"}}
                ]}],
                "PreToolUse": [{"hooks": [
                    {"type":"command","command":"true","async":true,"timeoutSec":10}
                ]}],
                "SessionEnd": [{"hooks": [
                    {"type":"command","command":"true","async":true}
                ]}],
                "Stop": [{"hooks": [
                    {"type":"mcp_tool","server":"codex_apps","tool":"check"}
                ]}]
            }
        })
        .to_string(),
    );
    let mut package = load_agent_plugin(root).unwrap();
    assert_eq!(package.diagnostics.len(), 1);
    assert!(package.diagnostics[0].contains("account connector"));
    let key = "codex.review".parse().unwrap();
    let adapter = AgentPluginAdapter::new(&key, &mut package, &[], &BTreeMap::new(), true).unwrap();
    let manifest = adapter.manifest();
    assert!(
        manifest
            .hooks
            .contains(HookSubscription::USER_PROMPT_SUBMIT)
    );
    assert_eq!(manifest.hook_handlers.len(), 3);
    assert!(
        manifest
            .hook_handlers
            .iter()
            .find(|handler| handler.id == "PreToolUse-0")
            .unwrap()
            .is_async
    );
    assert!(
        !manifest
            .hook_handlers
            .iter()
            .find(|handler| handler.id == "SessionEnd-0")
            .unwrap()
            .is_async
    );
}

#[test]
fn deepseek_imports_ordered_literal_hook_adapter_rows() {
    use agena_plugin_sdk::rpc::method::HOOK_IMPORTED_CLAUDE_STOP;
    use agena_plugin_sdk::{HookSubscription, Plugin};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "package.json",
        r#"{"name":"@vendor/hooks","dsh":{"bundle":{"patch":["./base.yml","./overrides.yml"]}}}"#,
    );
    write(
        root,
        "base.yml",
        r#"- insert:
  - name: '@deepseek-ai/dsh-hooks-codex'
    config:
      configPath: ./codex.json
  - id: claude
    name: '@deepseek-ai/dsh-hooks-claude-code'
    config:
      configPath: ./obsolete.json
"#,
    );
    write(
        root,
        "overrides.yml",
        r#"- id: claude
  name: '@deepseek-ai/dsh-hooks-claude-code'
  config:
    configPath: ./claude.json
"#,
    );
    write(
        root,
        "codex.json",
        r#"{"hooks":{
            "PreCompact":[{"hooks":[{"type":"command","command":"echo compact"}]}],
            "Stop":[{"hooks":[{"type":"command","command":"echo stop","timeout":600}]}]
        }}"#,
    );
    write(
        root,
        "claude.json",
        r#"{"hooks":{
            "PostToolUseFailure":[{"hooks":[{"type":"command","command":"echo logged"}]}],
            "PreToolUse":[{"hooks":[{"type":"agent","prompt":"Check the operation","timeout":120}]}],
            "Stop":[{"hooks":[{"type":"agent","prompt":"Check stopping","timeout":45}]}],
            "SubagentStop":[{"hooks":[{"type":"agent","prompt":"Check child stopping","timeout":96}]}]
        }}"#,
    );
    let mut package = load_agent_plugin(root).unwrap();
    assert!(package.diagnostics.is_empty(), "{:?}", package.diagnostics);
    assert!(package.has_claude_stop_hooks());
    assert_eq!(
        package.agent_hook_timeout(HOOK_IMPORTED_CLAUDE_STOP),
        Some(Duration::from_secs(96))
    );
    let manifest = AgentPluginAdapter::new(
        &"vendor.hooks".parse().unwrap(),
        &mut package,
        &[],
        &BTreeMap::new(),
        true,
    )
    .unwrap()
    .manifest();
    assert_eq!(
        manifest.hooks,
        HookSubscription::INIT
            | HookSubscription::SHUTDOWN
            | HookSubscription::COMPACTION_BEFORE
            | HookSubscription::TOOL_FAILURE
            | HookSubscription::TOOL_BEFORE
            | HookSubscription::AGENT_STOP
    );
}

struct HookHost {
    workspace: PathBuf,
    allow: bool,
    calls: std::sync::Mutex<Vec<(Option<i64>, Option<i64>)>>,
    agent_checks: std::sync::Mutex<Vec<String>>,
    workflow_storage: std::sync::Mutex<BTreeMap<(i64, String, String), String>>,
    workflow_launches: std::sync::Mutex<Vec<(String, String)>>,
    workflow_child_active: std::sync::atomic::AtomicBool,
    transcripts: std::sync::Mutex<Option<(PathBuf, PathBuf)>>,
}

#[async_trait::async_trait]
impl agena_plugin_sdk::HostClient for HookHost {
    async fn log(&self, _: agena_plugin_sdk::host_api::LogLevel, _: String, _: Value) {}

    async fn publish_event(
        &self,
        _: agena_plugin_sdk::EventEnvelope,
    ) -> agena_plugin_sdk::Result<()> {
        unreachable!()
    }

    async fn subscribe_events(
        &self,
        _: agena_plugin_sdk::EventFilter,
    ) -> agena_plugin_sdk::Result<agena_plugin_sdk::host_api::EventSubscription> {
        unreachable!()
    }

    async fn read_config(&self, _: Option<String>) -> agena_plugin_sdk::Result<Value> {
        unreachable!()
    }

    async fn invoke_tool(
        &self,
        tool: String,
        input: Value,
    ) -> agena_plugin_sdk::Result<agena_plugin_sdk::ToolInvokeOutput> {
        assert_eq!(tool, "agena.agent.spawn");
        assert_eq!(input["profile"], "vendor.review.reviewer");
        assert!(input["prompt"].as_str().unwrap().contains("check changes"));
        self.workflow_launches.lock().unwrap().push((
            input["name"].as_str().unwrap().to_owned(),
            input["prompt"].as_str().unwrap().to_owned(),
        ));
        let mut output = agena_plugin_sdk::ToolInvokeOutput::text("spawned");
        output.payload = Some(json!({"agent_id": 21}));
        Ok(output)
    }

    async fn wait_imported_plugin_agent(
        &self,
        child: i64,
        _: u64,
    ) -> agena_plugin_sdk::Result<agena_plugin_sdk::ToolInvokeOutput> {
        assert_eq!(child, 21);
        Ok(agena_plugin_sdk::ToolInvokeOutput::text("Agena result"))
    }

    async fn find_imported_plugin_agent(
        &self,
        name: String,
    ) -> agena_plugin_sdk::Result<Option<agena_plugin_sdk::host_api::ImportedPluginAgentState>>
    {
        use agena_plugin_sdk::host_api::ImportedPluginAgentState;
        Ok(self
            .workflow_launches
            .lock()
            .unwrap()
            .iter()
            .any(|launch| launch.0 == name)
            .then(|| {
                if self
                    .workflow_child_active
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    ImportedPluginAgentState::Active
                } else {
                    ImportedPluginAgentState::Completed(21)
                }
            }))
    }

    async fn storage_get(
        &self,
        request: agena_plugin_sdk::host_api::HostStorageGetRequest,
    ) -> agena_plugin_sdk::Result<agena_plugin_sdk::host_api::HostStorageGetResponse> {
        use agena_plugin_sdk::host_api::{
            HostStorageGetResponse, HostStorageScope, HostStorageVisibility,
            current_host_callback_context,
        };
        assert_eq!(request.scope, HostStorageScope::Session);
        assert_eq!(request.visibility, HostStorageVisibility::Private);
        let session = current_host_callback_context().unwrap().session_id.unwrap();
        let key = (session, request.namespace, request.key);
        Ok(HostStorageGetResponse {
            value: self.workflow_storage.lock().unwrap().get(&key).cloned(),
        })
    }

    async fn storage_set(
        &self,
        request: agena_plugin_sdk::host_api::HostStorageSetRequest,
    ) -> agena_plugin_sdk::Result<()> {
        use agena_plugin_sdk::host_api::{
            HostStorageScope, HostStorageVisibility, current_host_callback_context,
        };
        assert_eq!(request.scope, HostStorageScope::Session);
        assert_eq!(request.visibility, HostStorageVisibility::Private);
        let session = current_host_callback_context().unwrap().session_id.unwrap();
        self.workflow_storage
            .lock()
            .unwrap()
            .insert((session, request.namespace, request.key), request.value);
        Ok(())
    }

    async fn evaluate_imported_plugin_prompt(
        &self,
        prompt: String,
    ) -> agena_plugin_sdk::Result<String> {
        if prompt.starts_with("Observe provider error") {
            self.agent_checks.lock().unwrap().push(prompt);
            return Ok(r#"{"ok":false,"reason":"the failed provider round was observed"}"#.into());
        }
        assert!(prompt.contains(r#""command":"echo original""#));
        Ok(r#"{"ok":false,"reason":"prompt blocked this command"}"#.into())
    }

    async fn cordis_llm_complete(
        &self,
        request: agena_plugin_sdk::host_api::CordisLlmRequest,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> agena_plugin_sdk::Result<agena_plugin_sdk::host_api::CordisLlmResponse> {
        let joined = request
            .messages
            .iter()
            .map(|msg| format!("{}:{}", msg.role, msg.text))
            .collect::<Vec<_>>()
            .join("|");
        Ok(agena_plugin_sdk::host_api::CordisLlmResponse {
            text: format!(
                "llm-reply(sys={:?},msgs={joined})",
                request.system.as_deref().unwrap_or("")
            ),
            stop_reason: "stop".into(),
            provider: request.provider.unwrap_or_else(|| "mock".into()),
            model: request.model.unwrap_or_else(|| "mock-model".into()),
            usage: Some(agena_plugin_sdk::host_api::CordisLlmUsage {
                input_tokens: 12,
                output_tokens: 7,
            }),
        })
    }

    async fn evaluate_imported_plugin_agent(
        &self,
        prompt: String,
    ) -> agena_plugin_sdk::Result<String> {
        let first = prompt.lines().next().unwrap();
        let deny = first.starts_with("Deny")
            || (first.starts_with("Retry") && self.agent_checks.lock().unwrap().is_empty());
        self.agent_checks.lock().unwrap().push(prompt);
        Ok(if deny {
            r#"{"ok":false,"reason":"Agent verifier rejected the action"}"#
        } else {
            r#"{"ok":true}"#
        }
        .into())
    }

    async fn call_imported_plugin_mcp(
        &self,
        server: String,
        tool: String,
        input: Value,
    ) -> agena_plugin_sdk::Result<agena_plugin_sdk::ToolInvokeOutput> {
        assert_eq!(
            server,
            agent_plugin_mcp_server_name("vendor.review", "gate")
        );
        if matches!(tool.as_str(), "child-start" | "child-stop") {
            self.agent_checks.lock().unwrap().push(input.to_string());
            let result = if tool == "child-start" {
                json!({"continue":false,"hookSpecificOutput":{
                    "hookEventName":"SubagentStart","additionalContext":"Use the child checklist."
                }})
            } else if input["active"] == "true" {
                json!({})
            } else {
                json!({"decision":"block","reason":"Finish the child checklist."})
            };
            let mut answer = agena_plugin_sdk::ToolInvokeOutput::text("checked");
            answer.payload = Some(json!({"content":[{"type":"text","text":result.to_string()}]}));
            return Ok(answer);
        }
        if tool == "check-expansion" {
            assert_eq!(input["command"], "review:explain");
            let result = if input["arguments"] == "denied" {
                json!({"decision":"block","reason":"this invocation was denied"})
            } else {
                json!({"hookSpecificOutput":{"hookEventName":"UserPromptExpansion","additionalContext":"Follow the project checklist."}})
            };
            let mut answer = agena_plugin_sdk::ToolInvokeOutput::text("checked");
            answer.payload = Some(json!({"content":[{"type":"text","text":result.to_string()}]}));
            return Ok(answer);
        }
        assert_eq!(tool, "check");
        assert_eq!(input["command"], "echo original");
        let mut answer = agena_plugin_sdk::ToolInvokeOutput::text("checked");
        answer.payload = Some(
            json!({"content": [{"type":"text","text":r#"{"decision":"block","reason":"MCP blocked this command"}"#}]}),
        );
        Ok(answer)
    }

    async fn imported_hook_transcripts(
        &self,
        _format: agena_plugin_sdk::host_api::HookTranscriptFormat,
        scope: agena_plugin_sdk::host_api::HookTranscriptScope,
        _callback_generation: Option<u64>,
    ) -> agena_plugin_sdk::Result<agena_plugin_sdk::host_api::ImportedHookTranscripts> {
        use agena_plugin_sdk::host_api::{
            HookTranscriptScope, ImportedHookFile, ImportedHookTranscripts,
        };
        let Some((session_path, agent_path)) = self.transcripts.lock().unwrap().clone() else {
            return Ok(ImportedHookTranscripts::default());
        };
        let owner: Arc<dyn Send + Sync> = Arc::new(());
        Ok(match scope {
            HookTranscriptScope::Current | HookTranscriptScope::Parent => ImportedHookTranscripts {
                transcript: Some(ImportedHookFile::new(session_path, owner)),
                agent: None,
            },
            HookTranscriptScope::ParentAndAgent => ImportedHookTranscripts {
                transcript: Some(ImportedHookFile::new(session_path, owner.clone())),
                agent: Some(ImportedHookFile::new(agent_path, owner)),
            },
        })
    }

    async fn prepare_process_sandbox(
        &self,
        _resources: agena_plugin_sdk::host_api::HostProcessResources,
    ) -> agena_plugin_sdk::Result<agena_plugin_sdk::host_api::HostPreparedProcessSandbox> {
        use agena_plugin_sdk::host_api::{
            HostPreparedProcessSandbox, HostProcessExecution, current_host_callback_context,
        };
        let context = current_host_callback_context().unwrap();
        assert_eq!(context.plugin_id.as_deref(), Some("vendor.review"));
        assert!(context.authority_token.is_some());
        self.calls
            .lock()
            .unwrap()
            .push((context.session_id, context.call_id));
        if !self.allow {
            return Err(PluginError::invalid_params("sandbox unavailable"));
        }
        let workspace = self.workspace.display().to_string();
        Ok(HostPreparedProcessSandbox {
            cwd: workspace.clone(),
            workspace_root: workspace,
            scope: "test-session".into(),
            additional_working_directories: Vec::new(),
            permission_snapshot: None,
            call_bound: false,
            policy: serde_json::to_value(agena_process::sandbox::SandboxPolicy {
                readonly_paths: vec![self.workspace.clone()],
                ..Default::default()
            })
            .unwrap(),
            execution: Some(HostProcessExecution {
                id: "run-42".into(),
                model: "test-model".into(),
            }),
        })
    }
}

async fn make_hook_host(
    root: &Path,
    enabled: bool,
    allow: bool,
) -> (Arc<crate::PluginHost>, Arc<HookHost>) {
    let probe = Arc::new(HookHost {
        workspace: root.canonicalize().unwrap(),
        allow,
        calls: Default::default(),
        agent_checks: Default::default(),
        workflow_storage: Default::default(),
        workflow_launches: Default::default(),
        workflow_child_active: Default::default(),
        transcripts: Default::default(),
    });
    let config = serde_json::from_value(json!({
        "list": {"vendor.review": {"package": {
            "kind": "agent_plugin",
            "path": root,
            "enable_hooks": enabled
        }}}
    }))
    .unwrap();
    let host = crate::PluginHost::new(crate::PluginHostBuildConfig {
        config,
        workspace_root: root.to_owned(),
        host_client: Some(probe.clone()),
        agena_version: env!("CARGO_PKG_VERSION").into(),
        static_plugins: Vec::new(),
        callback_base_url: None,
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap();
    (host, probe)
}

fn shell_input() -> agena_plugin_sdk::ToolBeforeInput {
    agena_plugin_sdk::ToolBeforeInput {
        tool: "agena.shell.exec".parse().unwrap(),
        session_id: 19,
        call_id: 7,
        workspace_root: "/the tool context workspace".into(),
        tool_cwd: None,
        tags: vec![],
        input: json!({"command":"echo original"}),
        permission_ask: None,
        title_override: None,
        metadata: Default::default(),
        additional_context: Vec::new(),
    }
}

#[tokio::test]
async fn imported_tasks_and_hooks_call_agena_without_a_source_process() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(root, CLAUDE_MANIFEST, r#"{"name":"review"}"#);
    write(
        root,
        "agents/reviewer.md",
        "---\nname: reviewer\ndescription: Review code\ntools: [Read, Grep, NotebookRead, TodoWrite]\ndisallowedTools: Write, Bash\n---\nReview the changes.",
    );
    let package = load_agent_plugin(root).unwrap();
    let role = &package.roles[0];
    assert_eq!(
        role.tool_allowlist
            .as_ref()
            .unwrap()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "agena.fs.grep",
            "agena.fs.read",
            "agena.notebook.read",
            "agena.task.todo_write"
        ]
    );
    assert!(role.tool_denials.contains("agena.shell.open"));
    assert!(role.tool_denials.contains("agena.fs.write"));
    assert!(package.diagnostics.is_empty(), "{:?}", package.diagnostics);
    write(
        root,
        "hooks/hooks.json",
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"prompt","prompt":"Check $ARGUMENTS"}]}]}}"#,
    );
    let (host, _) = make_hook_host(root, true, true).await;
    let tool = host.lookup_tool_for_name("vendor.review.run").unwrap();
    let result = host
        .invoke_tool(
            &tool,
            agena_plugin_sdk::ToolInvokeInput {
                tool_name: "run".into(),
                session_id: 19,
                call_id: 70,
                workspace_root: root.display().to_string(),
                caller: None,
                input: json!({"agent":"reviewer","prompt":"check changes"}),
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.output_text, "Agena result");
    assert!(
        host.dispatch_tool_before(shell_input(), None)
            .await
            .unwrap_err()
            .diagnostic_message()
            .contains("prompt blocked this command")
    );

    write(
        root,
        "hooks/hooks.json",
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"mcp_tool","server":"plugin:review:gate","tool":"check","input":{"command":"${tool_input.command}"}}]}]}}"#,
    );
    let (host, _) = make_hook_host(root, true, true).await;
    assert!(
        host.dispatch_tool_before(shell_input(), None)
            .await
            .unwrap_err()
            .diagnostic_message()
            .contains("MCP blocked this command")
    );
}

#[tokio::test]
async fn host_dispatch_binds_hook_authority_and_honors_opt_in_and_failure_policy() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, CLAUDE_MANIFEST, r#"{"name":"review"}"#);
    write(
        root,
        "hooks/hooks.json",
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo ignored","onFailure":"block"}]}]}}"#,
    );

    let (host, probe) = make_hook_host(root, false, false).await;
    assert_eq!(
        host.dispatch_tool_before(shell_input(), None)
            .await
            .unwrap()
            .input["command"],
        "echo original"
    );
    assert!(probe.calls.lock().unwrap().is_empty());

    let (host, probe) = make_hook_host(root, true, false).await;
    let mut unmatched = shell_input();
    unmatched.tool = "agena.fs.read".parse().unwrap();
    assert!(host.dispatch_tool_before(unmatched, None).await.is_ok());
    assert!(probe.calls.lock().unwrap().is_empty());

    let error = host
        .dispatch_tool_before(shell_input(), None)
        .await
        .unwrap_err();
    assert!(error.diagnostic_message().contains("PreToolUse hook"));
    assert_eq!(*probe.calls.lock().unwrap(), [(Some(19), Some(7))]);
}

#[tokio::test]
async fn claude_function_hooks_wrap_tool_execution_and_keep_commands_in_the_session() {
    use crate::host::ImportedToolCall;
    use agena_plugin_sdk::{CommandInvokeInput, SessionEndInput, SessionEndReason};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(
        &root,
        CLAUDE_MANIFEST,
        r#"{
        "name":"review",
        "userConfig":{"start":{"type":"number","default":4}}
    }"#,
    );
    write(
        &root,
        "hooks/hooks.json",
        r#"{
        "modules":["../mod/register.ts"],
        "hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[
            {"type":"mcp_tool","if":"Bash(echo original)","server":"plugin:review:gate","tool":"check","input":{"command":"${tool_input.command}"}}
        ]}]}
    }"#,
    );
    write(
        &root,
        "mod/count.ts",
        r#"
        export const increment = (count: number): number => count + 1;
    "#,
    );
    write(
        &root,
        "mod/register.ts",
        r#"
        import type { On } from 'claude-code';
        import { increment } from './count';
        export function register(on: On, options: { start: number }) {
            let count = options.start;
            on('session.start', async ($, e, next) => {
                await $.command.register({ name:'tally', description:'Show tool call count', aliases:['count'] });
                return next(e);
            });
            on('session.end', (_$, e, next) => next(e));
            on('tool.call', { tool: /^Bash$/ }, async ($, e, next) => {
                count = increment(count);
                const answer = await next({ ...e, command: 'echo rewritten' });
                if ((await next(e)).result !== answer.result) throw Error('continuation ran twice');
                throw Error('recover after actual execution');
            }).catch(async ($, e, next) => {
                if (!next.called) throw Error('lost continuation');
                const answer = await next(e);
                return { ...answer, context:[...answer.context, 'after:' + answer.text] };
            });
            on('classic.PreToolUse', { tool: 'Bash' }, async (_$, e, next) => {
                if (e.command === 'echo original') {
                    const answer = await next(e);
                    return { deny: 'wrapped:' + answer.deny };
                }
                const answer = await next({ ...e, command: e.command + ' classic' });
                return {...answer, additionalContext:['classic saw ' + e.command]};
            });
            on('command.run', { command:'tally' }, ($, e) => ({text:`${count}:${e.args}`}));
        }
    "#,
    );
    let package = load_agent_plugin(&root).unwrap();
    assert!(package.diagnostics.is_empty(), "{:?}", package.diagnostics);
    assert!(package.watch_paths.contains(&root.join("mod/count.ts")));
    let (host, probe) = make_hook_host(&root, true, true).await;
    let mut before = shell_input();
    before.input["command"] = json!("echo original");
    assert!(
        host.dispatch_tool_before(before, None)
            .await
            .unwrap_err()
            .diagnostic_message()
            .contains("wrapped:MCP blocked this command")
    );
    assert_eq!(*probe.calls.lock().unwrap(), [(Some(19), Some(7))]);

    let original = shell_input();
    let terminal_calls = AtomicUsize::new(0);
    let answer = host.run_imported_tool_call(ImportedToolCall {
        tool: &original.tool,
        session_id: 19,
        call_id: original.call_id,
        workspace_root: root.to_str().unwrap(),
        sandbox_enabled: true,
        subagent: false,
        input: json!({"command":"echo before"}),
    }, |_| true, |input| {
        let mut invocation = shell_input();
        invocation.input = input;
        let host = &host;
        let terminal_calls = &terminal_calls;
        async move {
            terminal_calls.fetch_add(1, Ordering::SeqCst);
            let edited = host.dispatch_tool_before(invocation, None).await.unwrap();
            assert_eq!(edited.input["command"], "echo rewritten classic");
            json!({"result":"real output","text":"real output","context":edited.additional_context})
        }
    }).await;
    assert_eq!(terminal_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        answer["context"],
        json!(["classic saw echo rewritten", "after:real output"])
    );
    assert!(host.command_catalog().is_empty());
    let scope = host.surface_catalog_for_session(Some(19));
    assert_eq!(scope.commands[0].command.slash.as_deref(), Some("/tally"));
    assert!(scope.commands[0].command.aliases.contains(&"count".into()));
    assert!(
        host.surface_catalog_for_session(Some(20))
            .commands
            .is_empty()
    );
    let invoke = |id| CommandInvokeInput {
        command_id: "tally".into(),
        session_id: Some(id),
        call_id: None,
        input: Value::Null,
        raw: "hello".into(),
        workspace_root: Some(root.display().to_string()),
        slash: Some("/tally".into()),
        prompt_context: String::new(),
        cordis_attachments: None,
    };
    assert_eq!(
        host.invoke_plugin_command_async("vendor.review", invoke(19))
            .await
            .unwrap()
            .summary,
        "5:hello"
    );
    let mut numeric = invoke(19);
    numeric.raw = "3".into();
    assert_eq!(
        host.invoke_plugin_command_async("vendor.review", numeric)
            .await
            .unwrap()
            .summary,
        "5:3"
    );
    assert!(
        host.invoke_plugin_command_async("vendor.review", invoke(20))
            .await
            .is_err()
    );
    let session_start = agena_plugin_sdk::SessionStartInput {
        session_id: 20,
        source: agena_plugin_sdk::SessionStartSource::Resume,
        workspace_root: root.display().to_string(),
        model: None,
        session_title: None,
        agent_type: None,
        resumed_cache: None,
    };
    host.prepare_imported_mod_session(&session_start, |_| true)
        .await;
    host.prepare_imported_mod_session(&session_start, |_| true)
        .await;
    assert_eq!(
        host.invoke_plugin_command_async("vendor.review", invoke(20))
            .await
            .unwrap()
            .summary,
        "4:hello"
    );
    host.broadcast_session_end(SessionEndInput {
        session_id: 19,
        reason: SessionEndReason::Clear,
    })
    .await;
    assert!(
        host.surface_catalog_for_session(Some(19))
            .commands
            .is_empty()
    );
    assert!(
        host.resolve_command_for_session(Some(20), "vendor.review", "tally")
            .is_some()
    );
}

#[tokio::test]
async fn claude_mod_tools_are_registered_and_invoked_per_session() {
    use crate::PluginScopeKey;
    use agena_plugin_sdk::{
        SessionEndInput, SessionEndReason, SessionStartInput, SessionStartSource, ToolInvokeInput,
    };

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(&root, CLAUDE_MANIFEST, r#"{"name":"review"}"#);
    write(&root, "hooks/hooks.json", r#"{"modules":["./tools.js"]}"#);
    write(
        &root,
        "hooks/tools.js",
        r#"
        export function register(on) {
            let count = 0;
            on('session.start', async ($, e, next) => {
                const spec = {
                    name: 'ticket', description: 'Look up a ticket', title: 'Ticket lookup',
                    inputSchema: {type:'object', properties:{id:{type:'string'}}, required:['id']},
                    isDeferred: false,
                };
                const {tool} = await $.tool.register(spec);
                if (tool !== 'mcp__review__ticket') throw Error('wrong tool name');
                await $.tool.register({name:'legacy', description:'Use the legacy handler'});
                return next(e);
            });
            on('tool.run', {tool:'mcp__review__ticket'}, (_$, e) =>
                ({result:{id:e.id, count:++count}, context:['private-' + e.id]}));
            on('tool.call', {tool:'mcp__review__legacy'}, () => ({result:'legacy result'}));
        }
        "#,
    );
    let package = load_agent_plugin(&root).unwrap();
    assert!(package.diagnostics.is_empty(), "{:?}", package.diagnostics);
    let (host, _) = make_hook_host(&root, true, true).await;
    let start = |session_id| SessionStartInput {
        session_id,
        workspace_root: root.display().to_string(),
        source: SessionStartSource::Startup,
        model: None,
        session_title: None,
        agent_type: None,
        resumed_cache: None,
    };
    let lookup = |session, name| {
        host.lookup_tool_for_name_scope(name, Some(&PluginScopeKey::session(session)))
    };
    let invoke = |session_id, name: &str, input| ToolInvokeInput {
        tool_name: name.into(),
        session_id,
        call_id: 7,
        workspace_root: root.display().to_string(),
        caller: None,
        input,
    };
    let name = "mcp__review__ticket";
    host.prepare_imported_mod_session(&start(19), |_| true)
        .await;
    let tool = lookup(19, name).expect("session registration");
    assert_eq!(tool.canonical_name(), "vendor.review.mcp__review__ticket");
    assert!(host.lookup_tool_for_name(name).is_none());
    assert!(lookup(20, name).is_none());
    assert!(
        host.invoke_tool(&tool, invoke(19, name, json!({"id":42})), None)
            .await
            .is_err()
    );
    assert!(
        host.invoke_tool(&tool, invoke(20, name, json!({"id":"T-20"})), None)
            .await
            .is_err()
    );
    let result = host
        .invoke_tool(&tool, invoke(19, name, json!({"id":"T-19"})), None)
        .await
        .unwrap();
    assert_eq!(result.title, "Ticket lookup");
    assert_eq!(result.payload, Some(json!({"id":"T-19","count":1})));
    assert_eq!(result.additional_context, ["private-T-19"]);

    host.prepare_imported_mod_session(&start(20), |_| true)
        .await;
    let second = lookup(20, name).unwrap();
    let result = host
        .invoke_tool(&second, invoke(20, name, json!({"id":"T-20"})), None)
        .await
        .unwrap();
    assert_eq!(result.payload, Some(json!({"id":"T-20","count":1})));

    let legacy = "mcp__review__legacy";
    let legacy_tool = lookup(19, legacy).unwrap();
    assert_eq!(
        host.invoke_tool(&legacy_tool, invoke(19, legacy, json!({})), None)
            .await
            .unwrap()
            .output_text,
        "legacy result"
    );
    host.broadcast_session_end(SessionEndInput {
        session_id: 19,
        reason: SessionEndReason::Clear,
    })
    .await;
    assert!(lookup(19, name).is_none());
    assert!(lookup(20, name).is_some());
    assert!(
        host.invoke_tool(&tool, invoke(19, name, json!({"id":"T-19"})), None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn claude_tool_check_exposes_the_host_verdict_and_pins_the_effective_call() {
    use crate::host::{ImportedToolCall, ImportedToolDecision};

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(&root, CLAUDE_MANIFEST, r#"{"name":"review"}"#);
    write(&root, "hooks/hooks.json", r#"{"modules":["./check.mjs"]}"#);
    write(
        &root,
        "hooks/check.mjs",
        r#"
        export function register(on) {
            on('tool.check', { tool: /^Bash$/, isReadOnly: true }, async ($, e, next) => {
                if (e.input.command === 'cat altered.txt') {
                    e.input.command = 'cat replacement.txt';
                    return next(e);
                }
                const baseline = await next({});
                if ((await next(e)).decision !== baseline.decision) throw Error('inconsistent continuation');
                if (e.input.command === 'cat missing.txt') {
                    return { decision: 'deny', reason: baseline.decision + ':' + e.tool_use_id };
                }
                if (e.input.command === 'cat review.txt') {
                    return { decision: 'ask', reason: e.approver };
                }
                return baseline;
            }).catch(() => ({ decision: 'deny', reason: 'immutable tool check input' }));
        }
        "#,
    );
    assert!(load_agent_plugin(&root).unwrap().diagnostics.is_empty());
    let (host, _) = make_hook_host(&root, true, true).await;
    let tool = shell_input().tool;
    let check = |command: &str, read_only| {
        host.run_imported_tool_check(
            ImportedToolCall {
                tool: &tool,
                session_id: 19,
                call_id: 7,
                workspace_root: root.to_str().unwrap(),
                subagent: false,
                sandbox_enabled: true,
                input: json!({"command": command}),
            },
            read_only,
            &ImportedToolDecision::Allow,
            |_| true,
        )
    };
    assert_eq!(check("cat pass.txt", true).await, None);
    assert_eq!(check("cat missing.txt", false).await, None);
    assert_eq!(
        check("cat missing.txt", true).await,
        Some(ImportedToolDecision::Deny {
            reason: Some("allow:7".into())
        })
    );
    assert_eq!(
        check("cat review.txt", true).await,
        Some(ImportedToolDecision::Ask {
            reason: Some("person".into())
        })
    );
    assert_eq!(
        check("cat altered.txt", true).await,
        Some(ImportedToolDecision::Deny {
            reason: Some("immutable tool check input".into())
        })
    );
}

#[tokio::test]
async fn claude_prompt_submit_preserves_ingress_and_the_continuation_result() {
    use crate::host::ImportedPromptSubmit;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(&root, CLAUDE_MANIFEST, r#"{"name":"prompts"}"#);
    write(&root, "hooks/hooks.json", r#"{"modules":["./prompt.mjs"]}"#);
    write(
        &root,
        "hooks/prompt.mjs",
        r#"
        export function register(on) {
            on('prompt.submit', {origin:{kind:'sdk'}}, async ($, e, next) => {
                if (e.text === 'drop') return {drop:'before admission'};
                if (e.text === 'forge') return next({...e, origin:{kind:'composer'}});
                if (e.command?.args === 'forge') return next({...e, command:{...e.command, args:'other'}});
                if (e.text === 'media') {
                    const id = e.attachments[0].id;
                    const file = await $.prompt.media({id, offset:3});
                    if (file.base64 !== 'ZGVm') return {drop:'media lookup failed'};
                    const result = await next({...e, context:['read scoped media']});
                    if ((await $.prompt.media({id})).deny === undefined) {
                        $.ui.log('media handle survived admission', {level:'error'});
                    }
                    return result;
                }
                const result = await next({
                    ...e, text:'rewritten ' + e.text, context:['model context'], attachments:[]
                });
                if (e.text === 'late') return {drop:'after admission'};
                result.text = 'altered receipt';
                result.context.push('late context');
                return result;
            }).catch(async ($, e, next) => next.called ? next(e) : {drop:'pinned ingress'});
            on('prompt.submit', {text:/^rewritten /}, ($, e, next) => {
                if (e.attachments[0].filename !== 'image.png') throw Error('attachment lost');
                return next({...e, context:[...e.context, 'second note']});
            });
            on('prompt.submit', {command:{name:'prompts:inspect', args:'src'}}, ($, e, next) => {
                return next({...e, context:[...e.context, 'command note']});
            });
        }
        "#,
    );
    assert!(load_agent_plugin(&root).unwrap().diagnostics.is_empty());
    let (host, _) = make_hook_host(&root, true, true).await;
    let calls = AtomicUsize::new(0);
    let media = |id: String, offset| -> futures_util::future::BoxFuture<'_, Value> {
        Box::pin(async move {
            if id == "item" && offset == 3 {
                json!({"base64":"ZGVm"})
            } else {
                panic!("unexpected read for {id} at {offset}");
            }
        })
    };
    let submit = |text: &str, enabled, command: Option<Value>| {
        let calls = &calls;
        let mut input = json!({
            "text":text, "origin":{"kind":"sdk"}, "wait":false,
            "attachments":[{"type":"image","filename":"image.png","id":"item"}],
        });
        if let Some(command) = command {
            input["command"] = command;
        }
        host.run_imported_prompt_submit(
            ImportedPromptSubmit {
                session_id: 19,
                workspace_root: root.to_str().unwrap(),
                model: "fixture",
                sandbox_enabled: true,
                input,
                media: Some(&media),
            },
            move |_| enabled,
            move |event| {
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    json!({"text":event["text"], "context":event["context"]})
                })
            },
        )
    };
    assert_eq!(submit("drop", true, None).await["drop"], "before admission");
    assert_eq!(submit("forge", true, None).await["drop"], "pinned ingress");
    let forged = json!({"name":"prompts:inspect","args":"forge"});
    assert_eq!(
        submit("/inspect forge", true, Some(forged)).await["drop"],
        "pinned ingress"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    for text in ["first", "late"] {
        let result = submit(text, true, None).await;
        assert_eq!(result["text"], format!("rewritten {text}"));
        assert_eq!(result["context"], json!(["model context", "second note"]));
    }
    let command = json!({"name":"prompts:inspect","args":"src"});
    let result = submit("/inspect src", true, Some(command)).await;
    assert_eq!(result["text"], "/inspect src");
    assert_eq!(result["context"], json!(["model context", "command note"]));
    assert_eq!(
        submit("media", true, None).await["context"],
        json!(["read scoped media"])
    );
    assert_eq!(submit("drop", false, None).await["text"], "drop");
    assert_eq!(calls.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn claude_function_hooks_require_opt_in_and_reject_imports_outside_the_package() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("package");
    write(&root, CLAUDE_MANIFEST, r#"{"name":"review"}"#);
    write(
        &root,
        "hooks/hooks.json",
        r#"{"modules":["./guard.js","./forbidden.ts","../../outside.js"]}"#,
    );
    write(
        &root,
        "hooks/guard.js",
        r#"
        export function register(on) {
            on('classic.PreToolUse', { tool: 'Bash' }, () => ({ deny: 'mod veto' }));
        }
    "#,
    );
    write(
        &root,
        "hooks/forbidden.ts",
        "import 'node:fs'; export const register = () => {};",
    );
    write(
        directory.path(),
        "outside.js",
        "throw Error('outside source was evaluated');",
    );
    let package = load_agent_plugin(&root).unwrap();
    assert!(
        package
            .diagnostics
            .iter()
            .any(|message| message.contains("outside its package"))
    );
    assert!(
        package
            .diagnostics
            .iter()
            .any(|message| message.contains("package-relative imports"))
    );
    let (host, _) = make_hook_host(&root, true, true).await;
    assert!(
        host.dispatch_tool_before(shell_input(), None)
            .await
            .unwrap_err()
            .diagnostic_message()
            .contains("mod veto")
    );
    let (host, _) = make_hook_host(&root, false, true).await;
    assert!(host.dispatch_tool_before(shell_input(), None).await.is_ok());
    assert!(!host.has_imported_tool_call_hooks(|_| true));
    let mods = host
        .plugins()
        .iter()
        .find_map(|plugin| plugin.agent_plugin())
        .unwrap();
    assert!(mods.mod_runtime().is_none());
    assert!(
        !mods
            .diagnostics
            .iter()
            .any(|message| message.contains("package-relative imports"))
    );
}

#[tokio::test]
async fn commands_dispatch_through_the_host_command_catalog() {
    use agena_plugin_sdk::{CommandHostEffect, CommandInvokeInput};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        CLAUDE_MANIFEST,
        r#"{"name":"review","commands":{"inspect":{"content":"Inspect [$0] in [$ARGUMENTS].","argumentHint":"[path]"}}}"#,
    );
    write(
        root,
        "commands/hidden.md",
        "Explicit commands replace this directory.",
    );
    let (host, _) = make_hook_host(root, false, false).await;
    assert_eq!(host.command_catalog().len(), 1);
    assert!(host.command_catalog()[0].submits_prompt);
    assert_eq!(
        host.command_catalog()[0].command.slash.as_deref(),
        Some("/vendor.review.inspect")
    );
    let result = host
        .invoke_plugin_command_async(
            "vendor.review",
            CommandInvokeInput {
                command_id: "inspect".into(),
                input: Value::Null,
                session_id: Some(19),
                call_id: None,
                workspace_root: None,
                slash: None,
                raw: "\"src/file name.rs\" --verbose".into(),
                prompt_context: String::new(),
                cordis_attachments: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        result.effects,
        [CommandHostEffect::InsertPrompt {
            prompt: "Inspect [src/file name.rs] in [\"src/file name.rs\" --verbose].".into()
        }]
    );
}

#[tokio::test]
async fn claude_prompt_expansion_guards_inline_skill_and_preserves_bound_context() {
    use agena_plugin_sdk::{CommandHostEffect, CommandInvokeInput, CommandStatus};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root, CLAUDE_MANIFEST, r#"{"name":"review"}"#);
    write(
        &root,
        "skills/explain/SKILL.md",
        "---\nname: explain\ndescription: Explain an item\n---\nExplain $ARGUMENTS using ${CLAUDE_SKILL_DIR}.",
    );
    write(
        &root,
        "skills/model-only/SKILL.md",
        "---\nname: model-only\nuser-invocable: false\n---\nBody",
    );
    write(&root, "bin/demo", "#!/bin/sh\nexit 0");
    write(
        &root,
        "hooks/hooks.json",
        r#"{"hooks":{"UserPromptExpansion":[{"matcher":"^review:explain$","hooks":[{
          "type":"mcp_tool","server":"plugin:review:gate","tool":"check-expansion",
          "input":{"command":"${command_name}","arguments":"${command_args}"}
        }]}]}}"#,
    );
    let (host, probe) = make_hook_host(&root, true, true).await;
    assert_eq!(
        host.agent_plugin_bin_dirs().collect::<Vec<_>>(),
        [&root.join("bin")]
    );
    assert_eq!(host.command_catalog().len(), 1);
    let input = |args: &str| CommandInvokeInput {
        command_id: "explain".into(),
        input: json!({"args":args}),
        session_id: Some(19),
        call_id: None,
        workspace_root: Some(root.display().to_string()),
        slash: None,
        raw: String::new(),
        prompt_context: "not from a hook".into(),
        cordis_attachments: None,
    };
    let blocked = host
        .invoke_plugin_command_async("vendor.review", input("denied"))
        .await
        .unwrap();
    assert_eq!(blocked.status, CommandStatus::Failed);
    assert_eq!(
        blocked.detail.as_deref(),
        Some("this invocation was denied")
    );
    assert!(blocked.effects.is_empty());
    let result = host
        .invoke_plugin_command_async("vendor.review", input("scope"))
        .await
        .unwrap();
    assert_eq!(
        result.effects,
        [CommandHostEffect::InsertPrompt {
            prompt: format!(
                "Explain scope using {}.\n\nFollow the project checklist.",
                root.join("skills/explain").display()
            ),
        }]
    );
    assert_eq!(
        *probe.calls.lock().unwrap(),
        [(Some(19), None), (Some(19), None)]
    );
}

#[tokio::test]
async fn claude_stop_failure_observes_provider_error_instead_of_resuming_the_turn() {
    use agena_plugin_sdk::AgentStopInput;

    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), CLAUDE_MANIFEST, r#"{"name":"review"}"#);
    write(
        dir.path(),
        "hooks/hooks.json",
        r#"{"hooks":{
          "StopFailure":[{"matcher":"rate_limit","hooks":[{"type":"prompt","prompt":"Observe provider error $ARGUMENTS"}]}],
          "Stop":[{"hooks":[{"type":"agent","prompt":"Deny a regular stop"}]}]
        }}"#,
    );
    let (host, probe) = make_hook_host(dir.path(), true, true).await;
    let result = host
        .dispatch_agent_stop(AgentStopInput {
            session_id: 19,
            execution_id: None,
            model: Some("fixture".into()),
            stop_hook_active: false,
            last_assistant_message: None,
            run_error: Some("The provider rate limited this request.".into()),
            provider_failure: Some(agena_failure::FailureCategory::RateLimited),
            subagent: None,
        })
        .await
        .unwrap();
    assert!(result.continue_with_message.is_none());
    let seen = probe.agent_checks.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].contains(r#""error":"rate_limit""#));
    assert!(seen[0].contains("The provider rate limited this request."));
}

#[tokio::test]
async fn imported_subagent_hooks_inject_context_and_continue_the_child() {
    use agena_plugin_sdk::{AgentStopInput, SubagentHookContext, SubagentStartInput};

    for (manifest, hook_session) in [(CLAUDE_MANIFEST, "19"), (CODEX_MANIFEST, "21")] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), manifest, r#"{"name":"review"}"#);
        write(
            dir.path(),
            "hooks/hooks.json",
            r#"{"hooks":{
              "SubagentStart":[{"matcher":"^review:reviewer$","hooks":[{
                 "type":"mcp_tool","server":"plugin:review:gate","tool":"child-start",
                 "input":{"session":"${session_id}","agent":"${agent_id}","role":"${agent_type}"}
              }]}],
              "SubagentStop":[{"matcher":"^review:reviewer$","hooks":[{
                 "type":"mcp_tool","server":"plugin:review:gate","tool":"child-stop",
                 "input":{"session":"${session_id}","agent":"${agent_id}","active":"${stop_hook_active}","result":"${last_assistant_message}"}
              }]}],
              "Stop":[{"hooks":[{"type":"command","command":"exit 2"}]}]
            }}"#,
        );
        let (host, probe) = make_hook_host(dir.path(), true, true).await;
        let start = host
            .dispatch_subagent_start(SubagentStartInput {
                session_id: 19,
                child_session_id: 21,
                name: "generated-task-name".into(),
                agent_type: Some("review:reviewer".into()),
                model: Some("fixture".into()),
            })
            .await
            .unwrap();
        assert_eq!(start, ["Use the child checklist."]);
        let stop = |active| AgentStopInput {
            session_id: 21,
            execution_id: Some("run-42".into()),
            model: Some("fixture".into()),
            stop_hook_active: active,
            last_assistant_message: Some("The child's answer".into()),
            run_error: None,
            provider_failure: None,
            subagent: Some(SubagentHookContext {
                parent_session_id: 19,
                agent_type: "review:reviewer".into(),
                name: None,
            }),
        };
        let first = host.dispatch_agent_stop(stop(false)).await.unwrap();
        assert_eq!(
            first.continue_with_message.as_deref(),
            Some("Finish the child checklist.")
        );
        assert!(
            host.dispatch_agent_stop(stop(true))
                .await
                .unwrap()
                .continue_with_message
                .is_none()
        );
        assert_eq!(
            *probe.calls.lock().unwrap(),
            [(Some(21), None), (Some(21), None), (Some(21), None)]
        );
        let seen = probe.agent_checks.lock().unwrap();
        let inputs = seen
            .iter()
            .map(|input| serde_json::from_str::<Value>(input).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            inputs,
            [
                json!({"session":hook_session,"agent":"21","role":"review:reviewer"}),
                json!({"session":hook_session,"agent":"21","active":"false","result":"The child's answer"}),
                json!({"session":hook_session,"agent":"21","active":"true","result":"The child's answer"}),
            ]
        );
    }
}

#[tokio::test]
async fn agent_hooks_dispatch_conditionally_and_scoped_once_retries_until_success() {
    use agena_plugin_sdk::CommandInvokeInput;

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(root, CLAUDE_MANIFEST, r#"{"name":"review"}"#);
    write(
        root,
        "commands/gate.md",
        r#"---
hooks:
  PreToolUse:
    - matcher: Bash
      hooks:
        - type: agent
          prompt: Retry the command check
          if: Bash(echo *)
          once: true
---
Inspect the command.
"#,
    );
    let (host, probe) = make_hook_host(root, true, true).await;
    host.dispatch_tool_before(shell_input(), None)
        .await
        .unwrap();
    assert!(probe.agent_checks.lock().unwrap().is_empty());
    host.invoke_plugin_command_async(
        "vendor.review",
        CommandInvokeInput {
            command_id: "gate".into(),
            session_id: Some(19),
            call_id: None,
            workspace_root: Some(root.display().to_string()),
            slash: None,
            raw: String::new(),
            prompt_context: String::new(),
            cordis_attachments: None,
            input: Value::Null,
        },
    )
    .await
    .unwrap();

    let mut unmatched = shell_input();
    unmatched.input = json!({"command":"pwd"});
    host.dispatch_tool_before(unmatched, None).await.unwrap();
    let mut other_session = shell_input();
    other_session.session_id = 20;
    host.dispatch_tool_before(other_session, None)
        .await
        .unwrap();
    assert!(probe.agent_checks.lock().unwrap().is_empty());

    let error = host
        .dispatch_tool_before(shell_input(), None)
        .await
        .unwrap_err();
    assert!(
        error
            .diagnostic_message()
            .contains("Agent verifier rejected the action")
    );
    host.dispatch_tool_before(shell_input(), None)
        .await
        .unwrap();
    host.dispatch_tool_before(shell_input(), None)
        .await
        .unwrap();
    let calls = probe.agent_checks.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].contains(r#""hook_event_name":"PreToolUse""#));
    assert!(calls[1].contains(r#""command":"echo original""#));
}

#[tokio::test]
async fn imported_workflow_uses_agenas_agent_bridge_and_returns_javascript_result() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(root, CLAUDE_MANIFEST, r#"{"name":"review"}"#);
    write(root, "agents/reviewer.md", "Review the changes.");
    write(
        root,
        "workflows/check.js",
        r#"
export const meta = { name: "check", description: "Check the changes" };
phase("Review");
const values = await pipeline(args.targets, target =>
  agent("check changes in " + target, {agent: "reviewer"}));
return {values, isolated: typeof require === "undefined" && typeof process === "undefined"};
"#,
    );
    let (host, probe) = make_hook_host(root, true, false).await;
    let tool = host.lookup_tool_for_name("vendor.review.workflow").unwrap();
    let invoke = |session_id, call_id, input| {
        host.invoke_tool(
            &tool,
            agena_plugin_sdk::ToolInvokeInput {
                tool_name: "workflow".into(),
                session_id,
                call_id,
                workspace_root: root.display().to_string(),
                caller: None,
                input,
            },
            None,
        )
    };
    let result = invoke(
        19,
        70,
        json!({"name":"check","args":{"targets":["src","docs"]}}),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&result.output_text).unwrap(),
        json!({"values":["Agena result","Agena result"],"isolated":true})
    );
    let run_id = result.metadata["runId"].clone();
    let old_first = probe.workflow_launches.lock().unwrap()[0].0.clone();
    let namespace = format!(
        "claude.workflow.run.{}",
        uuid::Uuid::parse_str(&run_id).unwrap().simple()
    );
    // Simulate interruption after the host committed a child and before the
    // JS runner saved its value.
    {
        let mut storage = probe.workflow_storage.lock().unwrap();
        let journal = storage.get_mut(&(19, namespace, "check".into())).unwrap();
        let mut value: Value = serde_json::from_str(journal).unwrap();
        value["calls"][0]["outcome"] = json!({"pending":{"name":old_first}});
        *journal = value.to_string();
    }
    let resume = json!({"name":"check","resumeFromRunId":run_id});
    probe
        .workflow_child_active
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        invoke(19, 71, resume.clone())
            .await
            .unwrap_err()
            .diagnostic_message()
            .contains("still active")
    );
    probe
        .workflow_child_active
        .store(false, std::sync::atomic::Ordering::Relaxed);
    let replayed = invoke(19, 72, resume.clone()).await.unwrap();
    assert_eq!(replayed.output_text, result.output_text);
    assert_eq!(probe.workflow_launches.lock().unwrap().len(), 2);
    assert!(
        invoke(20, 73, resume)
            .await
            .unwrap_err()
            .diagnostic_message()
            .contains("no saved run")
    );
    invoke(
        19,
        74,
        json!({"name":"check", "resume":true, "args":{"targets":["src","tests","tools"]}}),
    )
    .await
    .unwrap();
    let launches = probe.workflow_launches.lock().unwrap();
    assert_eq!(launches.len(), 4);
    assert!(launches[2].1.ends_with("check changes in tests"));
    assert!(launches[3].1.ends_with("check changes in tools"));
}

#[test]
fn remote_mcp_transports_are_preserved_when_importing() {
    use super::mcp::AgentPluginMcpTransport;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        CLAUDE_MANIFEST,
        r#"{"name":"review","mcpServers":{
        "legacy":{"type":"sse","url":"https://fixture.invalid/events"},
        "socket":{"type":"ws","url":"wss://fixture.invalid/socket"}
    }}"#,
    );
    let plugin = load_agent_plugin(root).unwrap();
    let context = AgentPluginContext {
        root,
        data: root,
        workspace: root,
        environment: &[],
        user_config_env: &BTreeMap::new(),
    };
    for (server, transport, url) in [
        (
            "legacy",
            AgentPluginMcpTransport::Sse,
            "https://fixture.invalid/events",
        ),
        (
            "socket",
            AgentPluginMcpTransport::WebSocket,
            "https://fixture.invalid/socket",
        ),
    ] {
        let server = plugin
            .mcp_servers
            .iter()
            .find(|entry| entry.name == server)
            .unwrap();
        let ResolvedAgentPluginMcp::Http {
            url: actual_url,
            transport: actual_transport,
            ..
        } = server.resolve(&context).unwrap()
        else {
            panic!("expected a remote MCP server");
        };
        assert_eq!((actual_url.as_str(), actual_transport), (url, transport));
    }
}

#[test]
fn deepseek_lsp_rows_use_the_agenas_lsp_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        "package.json",
        r#"{"name":"@fixture/lsp","dsh":{"bundle":{"patch":"patch.yml"}}}"#,
    );
    write(
        root,
        "patch.yml",
        r#"
- insert:
    - id: fixture-lsp
      name: "@deepseek-ai/dsh-lsp-stdio"
      config:
        servers:
          typescript:
            command: !!js "'typescript-' + 'language-server'"
            args: [--stdio]
            extensionToLanguage: {".ts": typescript, ".tsx": typescriptreact}
            configuration: {typescript: {validate: true}}
            shutdownTimeoutMs: 4000
"#,
    );
    let package = load_agent_plugin(root).unwrap();
    assert!(package.diagnostics.is_empty(), "{:?}", package.diagnostics);
    let [server] = package.lsp_servers.as_slice() else {
        panic!("expected one LSP server")
    };
    assert_eq!(server.language_ids["ts"], "typescript");
    assert_eq!(server.language_ids["tsx"], "typescriptreact");
    assert_eq!(server.shutdown_timeout, Some(4000));
    assert_eq!(
        server.settings.as_ref().unwrap()["typescript"]["validate"],
        true
    );
    let context = AgentPluginContext {
        root,
        data: root,
        workspace: root,
        environment: &[],
        user_config_env: &BTreeMap::new(),
    };
    let launch = server.resolve(&context).unwrap();
    assert_eq!(launch.command, "typescript-language-server");
    assert_eq!(launch.args, ["--stdio"]);
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires a working sandbox launcher in AGENA_TEST_SANDBOX_LAUNCHER"]
async fn sandboxed_command_hook_round_trips_tool_payload_through_host() {
    let launcher = std::env::var_os("AGENA_TEST_SANDBOX_LAUNCHER").expect("sandbox launcher");
    agena_process::sandbox::configure_launcher(PathBuf::from(launcher)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, CLAUDE_MANIFEST, r#"{"name":"review"}"#);
    write(root, "hooks/hooks.json", json!({
        "hooks": {"PreToolUse": [{
            "matcher":"Bash",
            "hooks":[{"type":"command","command":"sh","args":["${CLAUDE_PLUGIN_ROOT}/hook.sh"],"onFailure":"block"}]
        }]}
    }).to_string());
    write(
        root,
        "hook.sh",
        r#"IFS= read -r payload || :
case "$payload" in *'"tool_name":"Bash"'*) ;; *) exit 3 ;; esac
case "$payload" in *'"turn_id":"run-42"'*) ;; *) exit 4 ;; esac
case "$payload" in *'"command":"echo original"'*) ;; *) exit 5 ;; esac
printf '%s' '{"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{"command":"echo changed"},"additionalContext":"hook ran"}}'
"#,
    );
    let (host, probe) = make_hook_host(root, true, true).await;
    let output = host
        .dispatch_tool_before(shell_input(), None)
        .await
        .unwrap();
    assert_eq!(output.input["command"], "echo changed");
    assert_eq!(output.additional_context, ["hook ran"]);
    assert_eq!(*probe.calls.lock().unwrap(), [(Some(19), Some(7))]);
}

#[tokio::test]
async fn claude_function_hooks_receive_transcript_paths_for_session_and_subagent() {
    if !agena_process::sandbox::enabled() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("package");
    let session_transcript = directory.path().join("session.jsonl");
    let agent_transcript = directory.path().join("agent.jsonl");
    std::fs::write(&session_transcript, "{\"role\":\"user\"}\n").unwrap();
    std::fs::write(&agent_transcript, "{\"role\":\"assistant\"}\n").unwrap();
    write(&root, CLAUDE_MANIFEST, r#"{"name":"review"}"#);
    write(&root, "hooks/hooks.json", r#"{"modules":["./transcript.mjs"]}"#);
    write(
        &root,
        "hooks/transcript.mjs",
        r#"
        export function register(on) {
            on('classic.SessionStart', async (_$, event, next) => {
                const seen = event.transcript_path;
                const result = await next({ ...event, transcript_path: '/forged' });
                return { ...result, additionalContext: [`session:${seen}`] };
            });
            on('classic.SubagentStop', async (_$, event, next) => {
                const result = await next({ ...event, transcript_path: '/forged', agent_transcript_path: '/forged' });
                return {
                    ...result,
                    block: `parent=${event.transcript_path}|agent=${event.agent_transcript_path}`,
                };
            });
        }
        "#,
    );
    let (host, probe) = make_hook_host(&root, true, true).await;
    *probe.transcripts.lock().unwrap() = Some((session_transcript.clone(), agent_transcript.clone()));

    let start = host
        .dispatch_session_start(agena_plugin_sdk::SessionStartInput {
            session_id: 19,
            workspace_root: root.to_string_lossy().into_owned(),
            model: Some("test-model".into()),
            source: agena_plugin_sdk::SessionStartSource::Startup,
            agent_type: None,
            session_title: None,
            resumed_cache: None,
        })
        .await
        .unwrap();
    assert_eq!(
        start.additional_context.as_deref(),
        Some(format!("session:{}", session_transcript.display()).as_str())
    );

    let stop = host
        .dispatch_agent_stop(agena_plugin_sdk::AgentStopInput {
            session_id: 21,
            execution_id: Some("run-1".into()),
            model: Some("test-model".into()),
            stop_hook_active: false,
            last_assistant_message: Some("done".into()),
            run_error: None,
            provider_failure: None,
            subagent: Some(agena_plugin_sdk::SubagentHookContext {
                parent_session_id: 19,
                agent_type: "review:reviewer".into(),
                name: None,
            }),
        })
        .await
        .unwrap();
    assert_eq!(
        stop.continue_with_message.as_deref(),
        Some(
            format!(
                "parent={}|agent={}",
                session_transcript.display(),
                agent_transcript.display()
            )
            .as_str()
        )
    );
}

#[test]
fn claude_agent_memory_and_background_options_parse_cleanly() {
    use super::ClaudeAgentMemoryScope;

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(root, CLAUDE_MANIFEST, r#"{"name":"roles"}"#);
    write(
        root,
        "agents/proj.md",
        "---\nname: proj\nmemory: project\nbackground: true\n---\nProject worker.",
    );
    write(
        root,
        "agents/local-worker.md",
        "---\nname: local-worker\nmemory: local\nbackground: false\n---\nLocal worker.",
    );
    write(
        root,
        "agents/user-worker.md",
        "---\nname: user-worker\nmemory: user\n---\nUser worker.",
    );
    write(
        root,
        "agents/invalid.md",
        "---\nname: invalid\nmemory: cloud\nbackground: nope\n---\nInvalid worker.",
    );
    let package = load_agent_plugin(root).unwrap();
    let role = |name: &str| {
        package
            .roles
            .iter()
            .find(|role| role.name == name)
            .unwrap()
    };
    assert_eq!(role("proj").claude_options.memory, Some(ClaudeAgentMemoryScope::Project));
    assert!(role("proj").claude_options.background);
    assert_eq!(role("proj").claude_options.role_name, "proj");
    assert_eq!(
        role("local-worker").claude_options.memory,
        Some(ClaudeAgentMemoryScope::Local)
    );
    assert!(!role("local-worker").claude_options.background);
    assert_eq!(
        role("user-worker").claude_options.memory,
        Some(ClaudeAgentMemoryScope::User)
    );
    assert_eq!(role("invalid").claude_options.memory, None);
    assert!(!role("invalid").claude_options.background);
    assert!(
        package
            .diagnostics
            .iter()
            .any(|msg| msg.contains("memory must be")),
        "{:?}",
        package.diagnostics
    );
    assert!(
        package
            .diagnostics
            .iter()
            .any(|msg| msg.contains("background must be a boolean")),
        "{:?}",
        package.diagnostics
    );
    assert!(
        !package
            .diagnostics
            .iter()
            .any(|msg| msg.contains("has no Agena adapter")),
        "{:?}",
        package.diagnostics
    );
}

#[tokio::test]
async fn claude_teammate_idle_and_mod_tool_offer_describe_content_is_end() {
    use agena_plugin_sdk::{
        AgentStopInput, SessionStartInput, SessionStartSource, SubagentHookContext,
        TOOL_END_TURN, ToolDefinitionInput, ToolInvokeInput, ToolKey,
    };

    const PNG_2X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DA8J8BAAf/Af8Bf4mnAAAAAElFTkSuQmCC";
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(&root, CLAUDE_MANIFEST, r#"{"name":"ext"}"#);
    write(&root, "hooks/hooks.json", r#"{"modules":["./ext.mjs"]}"#);
    write(
        &root,
        "hooks/ext.mjs",
        &format!(
            r#"
            export function register(on) {{
                on('session.start', async ($, e, next) => {{
                    await $.tool.register({{ name: 'preview', description: 'Original preview' }});
                    await $.tool.register({{ name: 'secret', description: 'Hidden tool' }});
                    return next(e);
                }});
                on('tool.offer', ($, e, next) => {{
                    if (e.tool === 'mcp__ext__secret') return {{ offer: false }};
                    return next(e);
                }});
                on('tool.describe', {{ tool: 'mcp__ext__preview' }}, () => ({{
                    description: 'Rewritten preview summary',
                    whenToUse: 'Use for image previews',
                }}));
                on('tool.run', {{ tool: 'mcp__ext__preview' }}, () => ({{
                    content: [
                        {{ type: 'text', text: 'preview ready' }},
                        {{ type: 'image', mimeType: 'image/png', data: '{PNG_2X1}', filename: 'shot.png' }},
                    ],
                    isEnd: true,
                }}));
                on('classic.TeammateIdle', {{ teammate_name: 'worker-alpha' }}, (_$, event) => {{
                    if (event.stop_hook_active) return {{}};
                    return {{ block: `keep going ${{event.teammate_name}} in ${{event.team_name}}` }};
                }});
            }}
            "#
        ),
    );
    let package = load_agent_plugin(&root).unwrap();
    assert!(package.diagnostics.is_empty(), "{:?}", package.diagnostics);
    assert!(package.has_claude_stop_hooks());
    let (host, _) = make_hook_host(&root, true, true).await;
    host.prepare_imported_mod_session(
        &SessionStartInput {
            session_id: 19,
            workspace_root: root.display().to_string(),
            source: SessionStartSource::Startup,
            model: None,
            session_title: None,
            agent_type: None,
            resumed_cache: None,
        },
        |_| true,
    )
    .await;

    let preview_key = ToolKey::new("vendor.ext".parse().unwrap(), "mcp__ext__preview").unwrap();
    let secret_key = ToolKey::new("vendor.ext".parse().unwrap(), "mcp__ext__secret").unwrap();
    let patched = host
        .dispatch_tool_definitions_for_enabled(
            vec![
                ToolDefinitionInput {
                    tool: preview_key.clone(),
                    summary: "Original preview".into(),
                    help: None,
                    input_schema: json!({"type":"object"}),
                    hidden: false,
                },
                ToolDefinitionInput {
                    tool: secret_key,
                    summary: "Hidden tool".into(),
                    help: None,
                    input_schema: json!({"type":"object"}),
                    hidden: false,
                },
            ],
            Some(19),
            Some(root.display().to_string()),
            |_| true,
        )
        .await;
    let preview_def = patched[0].as_ref().unwrap();
    assert!(!preview_def.hidden);
    assert_eq!(preview_def.summary, "Rewritten preview summary");
    assert!(
        preview_def
            .help
            .as_deref()
            .is_some_and(|help| help.contains("Use for image previews"))
    );
    assert!(patched[1].as_ref().unwrap().hidden);

    let tool = host
        .lookup_tool_for_name_scope("mcp__ext__preview", Some(&crate::PluginScopeKey::session(19)))
        .unwrap();
    let output = host
        .invoke_tool(
            &tool,
            ToolInvokeInput {
                tool_name: "mcp__ext__preview".into(),
                session_id: 19,
                call_id: 7,
                workspace_root: root.display().to_string(),
                caller: None,
                input: json!({}),
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(output.output_text, "preview ready");
    assert_eq!(output.attachments.len(), 1);
    assert_eq!(output.attachments[0].mime, "image/png");
    assert_eq!(output.attachments[0].width, Some(2));
    assert_eq!(output.attachments[0].height, Some(1));
    assert_eq!(
        output.metadata.get(TOOL_END_TURN).map(String::as_str),
        Some("true")
    );

    let stop = |active| AgentStopInput {
        session_id: 21,
        execution_id: Some("run-1".into()),
        model: Some("fixture".into()),
        stop_hook_active: active,
        last_assistant_message: Some("idle".into()),
        run_error: None,
        provider_failure: None,
        subagent: Some(SubagentHookContext {
            parent_session_id: 19,
            agent_type: "ext:worker".into(),
            name: Some("worker-alpha".into()),
        }),
    };
    let first = host.dispatch_agent_stop(stop(false)).await.unwrap();
    assert_eq!(
        first.continue_with_message.as_deref(),
        Some("keep going worker-alpha in ext")
    );
    let second = host.dispatch_agent_stop(stop(true)).await.unwrap();
    assert!(second.continue_with_message.is_none());
}

#[tokio::test]
async fn cordis_step_activated_tools_llm_service_custom_subagent_provider_and_node_builtins() {
    use agena_plugin_sdk::{CommandInvokeInput, ToolDefinitionInput, ToolInvokeInput, ToolKey};

    const PNG_2X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DA8J8BAAf/Af8Bf4mnAAAAAElFTkSuQmCC";
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    write(
        &root,
        "package.json",
        r#"{"name":"@fixture/cordis-wave2","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    );
    write(
        &root,
        "bundle.yml",
        r#"
- id: subagent
  config: {maxDepth: 2, maxActiveSubagents: 2}
- insert:
    - name: ./main.mjs
"#,
    );
    write(
        &root,
        "main.mjs",
        &format!(
            r#"
import {{ defineTool }} from '@deepseek-ai/dsh-tools';
import EventEmitter from 'node:events';
import util from 'node:util';
import assert from 'node:assert/strict';
import qs from 'node:querystring';
import os from 'node:os';

export const inject = ['commands', 'tools', 'subagents', 'llm'];
export function apply(ctx) {{
    let unlocked = false;
    const events = [];
    ctx.on('subagent/start', info => events.push(['start', info]));
    ctx.on('subagent/end', info => events.push(['end', info]));

    ctx.subagents.registerProvider({{
        name: 'custom-js',
        capabilities: {{ custom: true }},
        async startActivation(request) {{
            let disposed = false;
            return {{
                childId: request.childId ?? 'custom-default',
                name: request.name ?? 'custom-worker',
                result: Promise.resolve({{
                    stopReason: 'completed',
                    output: [{{
                        type: 'text',
                        text: `custom-handled:${{request.prompt.length}}:${{request.prompt[1]?.type}}`,
                    }}],
                }}),
                async dispose() {{ disposed = true; }},
            }};
        }},
    }});

    ctx.tools.register(
        defineTool({{
            name: 'gated_tool',
            description: 'Only active after unlock',
            deferLoading: true,
            parameters: {{ value: {{ type: 'string', required: true }} }},
            output: {{
                schema: {{ type: 'object' }},
                render: (_, value) => [{{ type: 'text', text: JSON.stringify(value) }}],
            }},
            async execute({{ value }}) {{
                return {{ ok: true, echoed: value }};
            }},
        }}),
        {{ when: () => unlocked }},
    );

    ctx.commands.register({{
        name: 'verify',
        description: 'Verify Wave 2 Cordis capabilities',
        async handler({{ agent, signal, rawInput }}) {{
            if (rawInput === 'unlock') {{
                unlocked = true;
                return {{ kind: 'success', text: 'unlocked' }};
            }}
            const emitter = new EventEmitter();
            let heard = '';
            emitter.once('ping', payload => {{ heard = payload; }});
            emitter.emit('ping', 'pong');
            assert.strictEqual(heard, 'pong');
            assert.deepStrictEqual({{ ...qs.parse(qs.stringify({{ a: '1', b: 'hello world' }})) }}, {{ a: '1', b: 'hello world' }});
            const cloned = structuredClone({{ nested: [1, 2], buf: new Uint8Array([9, 8]) }});
            assert.ok(util.isDeepStrictEqual(cloned.nested, [1, 2]));
            assert.ok(util.types.isUint8Array(cloned.buf));
            assert.strictEqual(os.EOL, '\n');

            const llmReply = await ctx.llm.complete({{
                system: 'helper',
                prompt: 'ping-llm',
                maxTokens: 64,
            }});
            const customRun = await ctx.subagents.startActivation({{
                parent: agent,
                signal,
                provider: 'custom-js',
                childId: 'custom-id-42',
                label: 'Custom JS provider run',
                prompt: [
                    {{ type: 'text', text: 'check media' }},
                    {{ type: 'image', mediaType: 'image/png', data: '{PNG_2X1}', filename: 'dot.png' }},
                ],
                delivery: 'caller',
            }});
            const customResult = await customRun.result;
            await customRun.dispose();
            return {{
                kind: 'success',
                text: JSON.stringify({{
                    formatted: util.format('%s:%d', 'node', 2),
                    platform: os.platform(),
                    llm: llmReply,
                    customProvider: ctx.subagents.getProvider('custom-js'),
                    customChildId: customRun.childId,
                    customResult,
                    events,
                }}),
            }};
        }},
    }});
}}
"#
        ),
    );
    let package = load_agent_plugin(&root).unwrap();
    assert!(package.diagnostics.is_empty(), "{:?}", package.diagnostics);
    let (host, _) = make_hook_host(&root, true, true).await;

    let gated_key = ToolKey::new("vendor.review".parse().unwrap(), "gated_tool").unwrap();
    let before = host
        .dispatch_tool_definitions_for_enabled(
            vec![ToolDefinitionInput {
                tool: gated_key.clone(),
                summary: "Only active after unlock".into(),
                help: None,
                input_schema: json!({"type":"object"}),
                hidden: false,
            }],
            Some(42),
            Some(root.display().to_string()),
            |_| true,
        )
        .await;
    assert!(before[0].as_ref().unwrap().hidden);

    let tool = host
        .lookup_tool_for_name_scope("vendor.review.gated_tool", None)
        .unwrap();
    let blocked = host
        .invoke_tool(
            &tool,
            ToolInvokeInput {
                tool_name: "gated_tool".into(),
                session_id: 42,
                call_id: 1,
                workspace_root: root.display().to_string(),
                caller: None,
                input: json!({"value":"early"}),
            },
            None,
        )
        .await;
    assert!(
        blocked
            .unwrap_err()
            .to_string()
            .contains("is not active in the current step")
    );

    let invoke = |raw: &str| CommandInvokeInput {
        command_id: "verify".into(),
        session_id: Some(42),
        call_id: None,
        input: Value::Null,
        raw: raw.into(),
        workspace_root: Some(root.display().to_string()),
        slash: Some("/verify".into()),
        prompt_context: String::new(),
        cordis_attachments: None,
    };
    let unlock = host
        .invoke_plugin_command_async("vendor.review", invoke("unlock"))
        .await
        .unwrap();
    assert_eq!(unlock.summary, "unlocked");

    let after = host
        .dispatch_tool_definitions_for_enabled(
            vec![ToolDefinitionInput {
                tool: gated_key,
                summary: "Only active after unlock".into(),
                help: None,
                input_schema: json!({"type":"object"}),
                hidden: false,
            }],
            Some(42),
            Some(root.display().to_string()),
            |_| true,
        )
        .await;
    assert!(!after[0].as_ref().unwrap().hidden);

    let ran = host
        .invoke_tool(
            &tool,
            ToolInvokeInput {
                tool_name: "gated_tool".into(),
                session_id: 42,
                call_id: 2,
                workspace_root: root.display().to_string(),
                caller: None,
                input: json!({"value":"unlocked-ok"}),
            },
            None,
        )
        .await
        .unwrap();
    assert!(ran.output_text.contains("unlocked-ok"));

    let verify = host
        .invoke_plugin_command_async("vendor.review", invoke("run"))
        .await
        .unwrap();
    let payload: Value = serde_json::from_str(&verify.summary).unwrap();
    assert_eq!(payload["formatted"], "node:2");
    assert_eq!(payload["platform"], "linux");
    assert_eq!(payload["llm"]["text"], "llm-reply(sys=\"helper\",msgs=user:ping-llm)");
    assert_eq!(payload["llm"]["usage"]["inputTokens"], 12);
    assert_eq!(payload["customProvider"]["name"], "custom-js");
    assert_eq!(payload["customChildId"], "custom-id-42");
    assert_eq!(
        payload["customResult"]["output"][0]["text"],
        "custom-handled:2:image"
    );
    assert_eq!(payload["events"][0][0], "start");
    assert_eq!(payload["events"][0][1]["id"], "custom-id-42");
    assert_eq!(payload["events"][0][1]["local"], false);
    assert_eq!(payload["events"][1][0], "end");
    assert_eq!(payload["events"][1][1]["stopReason"], "completed");
}
