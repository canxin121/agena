use super::*;
use crate::sdk::{
    CommandDefinition, CommandTarget, Plugin, PluginWebAssets, PluginWebContributions,
    PluginWebPage, PluginWebScope, SessionKind, SettingsContract,
};
use crate::web::PluginWebError;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct WebTestPlugin {
    manifest: PluginManifest,
    calls: Arc<AtomicUsize>,
    started: Arc<tokio::sync::Notify>,
    block: bool,
}

#[async_trait::async_trait]
impl Plugin for WebTestPlugin {
    fn manifest(&self) -> PluginManifest {
        self.manifest.clone()
    }

    async fn command_invoke(&self, input: CommandInvokeInput) -> crate::sdk::Result<CommandResult> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.started.notify_one();
        if self.block {
            std::future::pending::<()>().await;
        }
        Ok(CommandResult::succeeded("Completed").with_output(input.input))
    }
}

struct TestPackage(PathBuf);

impl Drop for TestPackage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    host: Arc<PluginHost>,
    plugin: Arc<LoadedPlugin>,
    calls: Arc<AtomicUsize>,
    started: Arc<tokio::sync::Notify>,
    _package: TestPackage,
}

async fn fixture(scope: PluginWebScope, block: bool) -> Fixture {
    let package = TestPackage(
        std::env::temp_dir().join(format!("agena-web-command-test-{}", uuid::Uuid::new_v4())),
    );
    std::fs::create_dir_all(package.0.join("web")).unwrap();
    std::fs::write(
        package.0.join("web/index.js"),
        "export function mount() { return { dispose() {} }; }",
    )
    .unwrap();
    let mut manifest = PluginManifest::new("test", "web", "1.0.0");
    manifest.commands = vec![CommandDefinition {
        id: "read".into(),
        title: "Read data".into(),
        group: "Web".into(),
        category: None,
        slash: None,
        aliases: vec![],
        applies_to: Default::default(),
        docs: Default::default(),
        input: SettingsContract::empty_object("Read", "Read plugin-owned data"),
        target: CommandTarget::Method {
            handler: "read".into(),
        },
    }];
    manifest.surface.web = Some(PluginWebContributions {
        assets: PluginWebAssets { root: "web".into() },
        pages: vec![PluginWebPage {
            id: "dashboard".into(),
            title: "Dashboard".into(),
            entry: "index.js".into(),
            commands: vec!["read".into()],
            scope,
            ..Default::default()
        }],
        ..Default::default()
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(tokio::sync::Notify::new());
    let implementation = WebTestPlugin {
        manifest: manifest.clone(),
        calls: calls.clone(),
        started: started.clone(),
        block,
    };
    let key: PluginKey = "test.web".parse().unwrap();
    let mut host = PluginHost::new_empty();
    let mutable = Arc::get_mut(&mut host).unwrap();
    let owner = mutable._host_handle.begin_plugin_instance(key.clone());
    let plugin = Arc::new(LoadedPlugin::new_with_scope(
        "inproc",
        crate::config::ConfiguredPlugin {
            package_root: Some(package.0.clone()),
            ..Default::default()
        },
        Arc::new(InProcessTransport::new(implementation)),
        owner,
        manifest,
        "test".into(),
        vec![],
    ));
    plugin.initialize_web_assets().await.unwrap();
    mutable
        ._host_handle
        .own_manifest_resources(&key, &plugin.manifest)
        .unwrap();
    mutable
        .statuses
        .set(crate::status::PluginStatus::initial(&key, "inproc"));
    mutable.plugins.push(plugin.clone());
    mutable.plugins_by_id.insert(key, plugin.clone());
    Fixture {
        host,
        plugin,
        calls,
        started,
        _package: package,
    }
}

fn input(value: serde_json::Value) -> CommandInvokeInput {
    CommandInvokeInput {
        command_id: "read".into(),
        input: value,
        session_id: None,
        call_id: None,
        workspace_root: None,
        slash: None,
        raw: String::new(),
    }
}

#[tokio::test]
async fn web_instance_commands_work_without_an_artificial_session() {
    let fixture = fixture(PluginWebScope::Instance, false).await;
    let pages = fixture.host.web_page_catalog();
    assert_eq!(pages.len(), 1);
    let page = &pages[0];
    assert_eq!(page.scope, PluginWebScope::Instance);
    assert!(page.entry_url.starts_with("/plugin-assets/test.web/"));
    let encoded = serde_json::to_value(page).unwrap();
    assert_eq!(encoded["style_urls"], json!([]));
    assert_eq!(encoded["settings"], false);
    let result = fixture
        .host
        .invoke_web_command(
            "test.web",
            "dashboard",
            &page.generation,
            input(json!({})),
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.title, "Read data");
    assert_eq!(result.output, Some(json!({})));
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn web_authorization_and_input_errors_never_enter_the_plugin() {
    let fixture = fixture(PluginWebScope::Instance, false).await;
    let page = fixture.host.web_page_catalog().remove(0);
    let host = &fixture.host;
    assert!(matches!(
        host.invoke_web_command(
            "test.web",
            "dashboard",
            "retired-generation",
            input(json!({})),
            None
        )
        .await,
        Err(PluginWebError::StaleGeneration)
    ));
    let mut undeclared = input(json!({}));
    undeclared.command_id = "private".into();
    assert!(matches!(
        host.invoke_web_command("test.web", "dashboard", &page.generation, undeclared, None)
            .await,
        Err(PluginWebError::Forbidden)
    ));
    assert!(matches!(
        host.invoke_web_command(
            "test.web",
            "missing",
            &page.generation,
            input(json!({})),
            None
        )
        .await,
        Err(PluginWebError::NotFound)
    ));
    let error = host
        .invoke_web_command(
            "test.web",
            "dashboard",
            &page.generation,
            input(json!({"unexpected": true})),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, PluginWebError::Plugin(error) if error.kind == PluginErrorKind::InvalidParams)
    );
    let mut injected = input(json!({}));
    injected.session_id = Some(42);
    assert!(matches!(
        host.invoke_web_command("test.web", "dashboard", &page.generation, injected, None)
            .await,
        Err(PluginWebError::InvalidInput(_))
    ));
    assert!(matches!(
        host.web_settings_inspect("test.web", "dashboard", &page.generation),
        Err(PluginWebError::Forbidden)
    ));
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn web_session_pages_require_project_context() {
    let fixture = fixture(PluginWebScope::Session, false).await;
    let page = fixture.host.web_page_catalog().remove(0);
    assert!(matches!(
        fixture
            .host
            .invoke_web_command(
                "test.web",
                "dashboard",
                &page.generation,
                input(json!({})),
                None
            )
            .await,
        Err(PluginWebError::InvalidInput(_))
    ));
    let mut scoped = input(json!({}));
    scoped.session_id = Some(42);
    scoped.workspace_root = Some("/project".into());
    fixture
        .host
        .invoke_web_command(
            "test.web",
            "dashboard",
            &page.generation,
            scoped,
            Some(SessionKind::Project),
        )
        .await
        .unwrap();
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn web_restart_and_rebind_invalidate_the_previous_generation() {
    let fixture = fixture(PluginWebScope::Instance, false).await;
    let old = fixture.host.web_page_catalog().remove(0);
    fixture
        .host
        .statuses
        .record_started(&fixture.plugin.key(), None, true);
    let current = fixture.host.web_page_catalog().remove(0);
    assert_ne!(old.generation, current.generation);
    assert_eq!(old.revision, current.revision);
    assert!(matches!(
        fixture
            .host
            .invoke_web_command(
                "test.web",
                "dashboard",
                &old.generation,
                input(json!({})),
                None
            )
            .await,
        Err(PluginWebError::StaleGeneration)
    ));
    let rebound = fixture
        .plugin
        .rebind_effect_scope(PluginEffectScope::new(fixture.plugin.key()));
    assert_ne!(fixture.plugin.web_generation, rebound.web_generation);
}

#[tokio::test]
async fn web_revocation_removes_assets_and_cancels_in_flight_calls() {
    let fixture = fixture(PluginWebScope::Instance, true).await;
    let page = fixture.host.web_page_catalog().remove(0);
    assert!(
        fixture
            .host
            .web_asset("test.web", &page.revision, "index.js")
            .is_ok()
    );
    assert!(matches!(
        fixture
            .host
            .web_asset("test.web", &page.revision, "../index.js"),
        Err(PluginWebError::NotFound)
    ));
    assert!(matches!(
        fixture
            .host
            .web_asset("test.web", "retired-revision", "index.js"),
        Err(PluginWebError::NotFound)
    ));
    let host = fixture.host.clone();
    let generation = page.generation.clone();
    let invocation = tokio::spawn(async move {
        host.invoke_web_command("test.web", "dashboard", &generation, input(json!({})), None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), fixture.started.notified())
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(2),
        fixture.plugin.effect_scope.dispose(),
    )
    .await
    .expect("page cancellation must release the effect lease");
    assert!(matches!(
        invocation.await.unwrap(),
        Err(PluginWebError::StaleGeneration)
    ));
    assert!(fixture.host.web_page_catalog().is_empty());
    assert!(matches!(
        fixture
            .host
            .web_asset("test.web", &page.revision, "index.js"),
        Err(PluginWebError::NotFound)
    ));
    assert!(matches!(
        fixture
            .host
            .invoke_web_command(
                "test.web",
                "dashboard",
                &page.generation,
                input(json!({})),
                None
            )
            .await,
        Err(PluginWebError::NotFound)
    ));
}
