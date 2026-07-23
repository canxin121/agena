//! Workspace dependency-graph invariants.
//!
//! This command intentionally obtains its input from Cargo rather than parsing
//! manifests itself, so renamed packages, target-specific dependencies, and
//! feature resolution continue to be interpreted by Cargo.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
struct Package {
    name: String,
    manifest_path: PathBuf,
    dependencies: Vec<Dependency>,
    targets: Vec<Target>,
}

#[derive(Debug, Deserialize)]
struct Dependency {
    name: String,
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

/// An edge that is forbidden once both sides are present in the workspace.
/// Keeping future-layer invariants here makes the checker become stricter as
/// each crate is introduced, without inventing temporary compatibility rules.
const FORBIDDEN_EDGES: &[(&str, &str)] = &[
    ("agena-domain", "tokio"),
    ("agena-domain", "reqwest"),
    ("agena-domain", "sea-orm"),
    ("agena-domain", "clap"),
    ("agena-domain", "ratatui"),
    ("agena-domain", "axum"),
    ("agena-provider", "agena"),
    ("agena-provider", "agena-storage"),
    ("agena-provider", "agena-tool"),
    ("agena-provider", "sea-orm"),
    ("agena-provider", "reqwest"),
    ("agena-provider", "clap"),
    ("agena-provider", "ratatui"),
    ("agena-tool", "agena"),
    ("agena-tool", "agena-provider"),
    ("agena-tool", "agena-storage"),
    ("agena-tool", "sea-orm"),
    ("agena-tool", "tokio"),
    ("agena-tool", "reqwest"),
    ("agena-tool", "clap"),
    ("agena-tool", "ratatui"),
    ("agena-tool", "axum"),
    ("agena-storage", "agena"),
    ("agena-storage", "agena-provider"),
    ("agena-storage", "agena-tool"),
    ("agena-storage", "sea-orm"),
    ("agena-storage", "tokio"),
    ("agena-storage", "reqwest"),
    ("agena-storage", "clap"),
    ("agena-storage", "ratatui"),
    ("agena-storage", "axum"),
    ("agena-runtime", "agena"),
    ("agena-application", "agena"),
    ("agena-application", "axum"),
    ("agena-application", "ratatui"),
    ("agena-application", "sea-orm"),
    ("agena-tui", "agena-api-server"),
    ("agena-tui", "agena"),
    ("agena-tui", "agena-application"),
    ("agena-tui", "agena-provider"),
    ("agena-tui", "agena-runtime"),
    ("agena-tui", "agena-storage"),
    ("agena-tui", "agena-tool"),
    ("agena-tui", "agena-client"),
    ("agena-tui", "sea-orm"),
    ("agena-tui", "clap"),
    ("agena-cli", "ratatui"),
    ("agena-cli", "agena-api-server"),
    ("agena-api", "agena-runtime"),
    ("agena-api", "agena-application"),
    ("agena-api", "agena-scheduler"),
    ("agena-api", "agena"),
    ("agena-client", "agena-domain"),
    ("agena-client", "agena-runtime"),
    ("agena-client", "agena-application"),
    ("agena-client", "agena"),
];

fn main() -> Result<()> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .context("failed to execute `cargo metadata`")?;
    if !output.status.success() {
        bail!(
            "`cargo metadata` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let metadata: Metadata =
        serde_json::from_slice(&output.stdout).context("Cargo returned invalid metadata JSON")?;
    let packages = metadata
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect::<BTreeMap<_, _>>();

    assert_terminal_binary(&packages)?;
    assert_default_member_boundary(&packages)?;
    assert_build_artifact_policy()?;
    assert_tui_presentation_asset_ownership()?;
    assert_domain_execution_selection_ownership()?;
    assert_domain_agent_profile_values_ownership()?;
    assert_provider_prompt_cache_ownership()?;
    assert_provider_http_utility_ownership()?;
    assert_provider_protocol_id_ownership()?;
    assert_provider_tool_stream_ownership()?;
    assert_provider_prompt_tool_envelope_ownership()?;
    assert_provider_prompt_tool_decoder_ownership()?;
    assert_bedrock_credential_sdk_leaf_ownership()?;
    assert_bedrock_signing_sdk_leaf_ownership()?;
    assert_bedrock_streaming_sdk_leaf_ownership()?;
    assert_google_adc_sdk_leaf_ownership()?;
    assert_provider_anthropic_text_wire_ownership()?;
    assert_provider_anthropic_wire_ownership()?;
    assert_provider_anthropic_thinking_ownership()?;
    assert_provider_gemini_thinking_ownership()?;
    assert_provider_gemini_usage_ownership()?;
    assert_provider_gemini_model_ownership()?;
    assert_provider_gemini_content_wire_ownership()?;
    assert_provider_gemini_request_wire_ownership()?;
    assert_provider_gemini_response_wire_ownership()?;
    assert_provider_gemini_live_response_wire_ownership()?;
    assert_provider_ollama_wire_ownership()?;
    assert_provider_ollama_usage_ownership()?;
    assert_provider_copilot_model_ownership()?;
    assert_provider_tool_mode_policy_ownership()?;
    assert_provider_openai_responses_wire_ownership()?;
    assert_provider_openai_chat_usage_ownership()?;
    assert_provider_openai_chat_response_format_ownership()?;
    assert_provider_openai_chat_reasoning_ownership()?;
    assert_provider_openai_chat_response_wire_ownership()?;
    assert_provider_openai_chat_tool_definition_ownership()?;
    assert_provider_openai_chat_request_wire_ownership()?;
    assert_runtime_provider_sse_ownership()?;
    assert_runtime_config_value_ownership()?;
    assert_domain_permission_decision_ownership()?;
    assert_runtime_project_path_ownership()?;
    assert_runtime_installation_id_ownership()?;
    assert_runtime_memory_plugin_ownership()?;
    assert_runtime_web_plugin_ownership()?;
    assert_memory_index_leaf_ownership()?;
    assert_tool_search_ownership()?;
    assert_tool_code_search_ownership()?;
    assert_tool_shell_contract_ownership()?;
    assert_legacy_monolith_deleted(&packages)?;
    assert_forbidden_edges(&packages)?;
    assert_no_textual_source_includes(&packages)?;
    Ok(())
}

fn assert_provider_prompt_cache_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let provider_cache = workspace.join("crates/agena-provider/src/prompt_cache_control.rs");
    let provider_source = fs::read_to_string(&provider_cache)
        .with_context(|| format!("read {}", provider_cache.display()))?;
    for required in [
        "pub struct PromptCacheControl",
        "pub fn select_cache_target_indices",
    ] {
        if !provider_source.contains(required) {
            bail!("provider prompt-cache control must retain `{required}`");
        }
    }
    let runtime_bootstrap_result =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/bootstrap_result.rs"))
            .context("read Runtime bootstrap result boundary")?;
    for required in [
        "pub struct RuntimeBootstrapResult",
        "pub fn application_services(&self) -> RuntimeApplicationServices",
        "pub fn shutdown(&self)",
    ] {
        if !runtime_bootstrap_result.contains(required) {
            bail!(
                "Runtime bootstrap result must retain application lifecycle capability `{required}`"
            );
        }
    }
    for forbidden in ["DatabaseConnection", "database_connection"] {
        if runtime_bootstrap_result.contains(forbidden) {
            bail!("Runtime bootstrap result must not return concrete database state `{forbidden}`");
        }
    }
    let runtime_root = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
        .context("read Runtime root visibility")?;
    if runtime_root.contains("pub mod runtime;")
        || runtime_root.contains("pub use runtime::{AgenaRuntime")
        || runtime_root.contains("pub use runtime::{RuntimeSnapshot")
    {
        bail!("Runtime must not expose concrete runtime/snapshot implementation types");
    }
    if workspace
        .join("crates/agena-runtime/src/provider/prompt_cache.rs")
        .exists()
    {
        bail!("Core must not retain prompt-cache control implementation");
    }
    Ok(())
}

fn assert_google_adc_sdk_leaf_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let leaf_manifest =
        fs::read_to_string(workspace.join("crates/agena-provider-google-auth/Cargo.toml"))
            .context("read Google ADC SDK leaf manifest")?;
    if !leaf_manifest.contains("name = \"agena-provider-google-auth\"")
        || !leaf_manifest.contains("gcp_auth = { workspace = true }")
    {
        bail!("Google ADC SDK leaf must own the gcp_auth dependency");
    }
    let leaf_source =
        fs::read_to_string(workspace.join("crates/agena-provider-google-auth/src/lib.rs"))
            .context("read Google ADC SDK leaf source")?;
    for required in [
        "pub async fn access_token",
        "gcp_auth::provider()",
        "GoogleAdcError",
    ] {
        if !leaf_source.contains(required) {
            bail!("Google ADC SDK leaf must retain `{required}`");
        }
    }

    let runtime_manifest = fs::read_to_string(workspace.join("crates/agena-runtime/Cargo.toml"))
        .context("read Runtime manifest for Google ADC ownership")?;
    if runtime_manifest.contains("gcp_auth =")
        || !runtime_manifest.contains("agena-provider-google-auth = { workspace = true }")
    {
        bail!("Runtime must consume the Google ADC leaf rather than gcp_auth directly");
    }
    let credential_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/credential.rs"))
            .context("read Runtime credential source")?;
    if credential_source.contains("gcp_auth::")
        || !credential_source.contains("google_adc_access_token")
    {
        bail!("Runtime credential logic must delegate Google SDK calls to its leaf adapter");
    }
    Ok(())
}

fn assert_memory_index_leaf_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let leaf_manifest = fs::read_to_string(workspace.join("crates/agena-memory-index/Cargo.toml"))
        .context("read memory-index leaf manifest")?;
    for required in [
        "name = \"agena-memory-index\"",
        "agena-storage = { workspace = true }",
        "tantivy = { workspace = true }",
    ] {
        if !leaf_manifest.contains(required) {
            bail!("memory-index leaf must retain `{required}`");
        }
    }
    let leaf_source = fs::read_to_string(workspace.join("crates/agena-memory-index/src/lib.rs"))
        .context("read memory-index leaf source")?;
    for required in [
        "pub struct MemorySearchDocument",
        "pub struct MemoryIndex",
        "pub fn replace_documents",
        "pub fn search",
        "MemoryDir::from_workspace",
    ] {
        if !leaf_source.contains(required) {
            bail!("memory-index leaf must retain `{required}`");
        }
    }
    let runtime_manifest = fs::read_to_string(workspace.join("crates/agena-runtime/Cargo.toml"))
        .context("read Runtime manifest for memory-index ownership")?;
    if runtime_manifest.contains("tantivy =")
        || !runtime_manifest.contains("agena-memory-index = { workspace = true }")
    {
        bail!("Runtime must consume the memory-index leaf rather than Tantivy directly");
    }
    if workspace
        .join("crates/agena-runtime/src/memory/index.rs")
        .exists()
    {
        bail!("Runtime must not retain the memory Tantivy index implementation");
    }
    let plugin_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/memory/plugin.rs"))
            .context("read Runtime memory plugin")?;
    if !plugin_source.contains("use agena_memory_index::{MemoryIndex, MemorySearchDocument};") {
        bail!("Runtime memory plugin must consume the memory-index leaf directly");
    }
    Ok(())
}

fn assert_bedrock_credential_sdk_leaf_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let leaf_manifest =
        fs::read_to_string(workspace.join("crates/agena-provider-bedrock-auth/Cargo.toml"))
            .context("read Bedrock credential SDK leaf manifest")?;
    for required in [
        "name = \"agena-provider-bedrock-auth\"",
        "aws-config = { workspace = true, default-features = true }",
        "aws-credential-types = { workspace = true }",
    ] {
        if !leaf_manifest.contains(required) {
            bail!("Bedrock credential SDK leaf must retain `{required}`");
        }
    }
    let leaf_source =
        fs::read_to_string(workspace.join("crates/agena-provider-bedrock-auth/src/lib.rs"))
            .context("read Bedrock credential SDK leaf source")?;
    for required in [
        "pub async fn resolve_credentials",
        "aws_config::defaults",
        "provide_credentials()",
        "pub fn static_credentials",
    ] {
        if !leaf_source.contains(required) {
            bail!("Bedrock credential SDK leaf must retain `{required}`");
        }
    }

    let runtime_manifest = fs::read_to_string(workspace.join("crates/agena-runtime/Cargo.toml"))
        .context("read Runtime manifest for Bedrock credential ownership")?;
    for forbidden in ["aws-config =", "aws-credential-types ="] {
        if runtime_manifest.contains(forbidden) {
            bail!("Runtime must not retain direct Bedrock credential SDK dependency `{forbidden}`");
        }
    }
    if !runtime_manifest.contains("agena-provider-bedrock-auth = { workspace = true }") {
        bail!("Runtime must consume the Bedrock credential SDK leaf");
    }
    for relative in [
        "crates/agena-runtime/src/provider/amazon_bedrock.rs",
        "crates/agena-runtime/src/provider/amazon_bedrock/bedrock_adapter.rs",
        "crates/agena-runtime/src/config/registry/auth_resolution.rs",
    ] {
        let source = fs::read_to_string(workspace.join(relative))
            .with_context(|| format!("read {relative}"))?;
        if source.contains("aws_config::") || source.contains("aws_credential_types::") {
            bail!("Runtime Bedrock credential consumer must use the SDK leaf: {relative}");
        }
    }
    Ok(())
}

fn assert_bedrock_signing_sdk_leaf_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let leaf_manifest =
        fs::read_to_string(workspace.join("crates/agena-provider-bedrock-signing/Cargo.toml"))
            .context("read Bedrock signing SDK leaf manifest")?;
    for required in [
        "name = \"agena-provider-bedrock-signing\"",
        "agena-provider-bedrock-auth = { workspace = true }",
        "aws-sigv4 = { workspace = true }",
    ] {
        if !leaf_manifest.contains(required) {
            bail!("Bedrock signing SDK leaf must retain `{required}`");
        }
    }
    let leaf_source =
        fs::read_to_string(workspace.join("crates/agena-provider-bedrock-signing/src/lib.rs"))
            .context("read Bedrock signing SDK leaf source")?;
    for required in [
        "pub fn signed_headers",
        "aws_sigv4",
        "apply_to_request_http1x",
        "BedrockSigningError",
    ] {
        if !leaf_source.contains(required) {
            bail!("Bedrock signing SDK leaf must retain `{required}`");
        }
    }

    let runtime_manifest = fs::read_to_string(workspace.join("crates/agena-runtime/Cargo.toml"))
        .context("read Runtime manifest for Bedrock signing ownership")?;
    if runtime_manifest.contains("aws-sigv4 =")
        || !runtime_manifest.contains("agena-provider-bedrock-signing = { workspace = true }")
    {
        bail!("Runtime must consume the Bedrock signing leaf rather than aws-sigv4 directly");
    }
    let runtime_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/amazon_bedrock.rs"))
            .context("read Runtime Bedrock signing source")?;
    if runtime_source.contains("aws_sigv4") || runtime_source.contains("signed_sigv4_headers") {
        bail!("Runtime Bedrock module must not retain the SigV4 signing implementation");
    }
    let adapter_source = fs::read_to_string(
        workspace.join("crates/agena-runtime/src/provider/amazon_bedrock/bedrock_adapter.rs"),
    )
    .context("read Runtime Bedrock adapter")?;
    if !adapter_source.contains("agena_provider_bedrock_signing::signed_headers") {
        bail!("Runtime Bedrock adapter must invoke the signing leaf");
    }
    Ok(())
}

fn assert_bedrock_streaming_sdk_leaf_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let leaf_manifest =
        fs::read_to_string(workspace.join("crates/agena-provider-bedrock-streaming/Cargo.toml"))
            .context("read Bedrock streaming SDK leaf manifest")?;
    for required in [
        "name = \"agena-provider-bedrock-streaming\"",
        "aws-smithy-eventstream = { workspace = true }",
        "aws-smithy-http = { workspace = true, features = [\"event-stream\"] }",
        "aws-smithy-types = { workspace = true, features = [\"http-body-1-x\"] }",
    ] {
        if !leaf_manifest.contains(required) {
            bail!("Bedrock streaming SDK leaf must retain `{required}`");
        }
    }
    let leaf_source =
        fs::read_to_string(workspace.join("crates/agena-provider-bedrock-streaming/src/lib.rs"))
            .context("read Bedrock streaming SDK leaf source")?;
    for required in [
        "pub fn decode_response",
        "BedrockAnthropicStreamDecodeError",
        "BedrockAnthropicStreamUnmarshaller",
        "Receiver::<Value, BedrockAnthropicStreamServiceError>::new",
    ] {
        if !leaf_source.contains(required) {
            bail!("Bedrock streaming SDK leaf must retain `{required}`");
        }
    }

    let runtime_manifest = fs::read_to_string(workspace.join("crates/agena-runtime/Cargo.toml"))
        .context("read Runtime manifest for Bedrock streaming ownership")?;
    for forbidden in [
        "aws-smithy-eventstream =",
        "aws-smithy-http =",
        "aws-smithy-types =",
    ] {
        if runtime_manifest.contains(forbidden) {
            bail!("Runtime must not retain direct Bedrock Smithy dependency `{forbidden}`");
        }
    }
    if !runtime_manifest.contains("agena-provider-bedrock-streaming = { workspace = true }") {
        bail!("Runtime must consume the Bedrock streaming SDK leaf");
    }
    for relative in [
        "crates/agena-runtime/src/provider/amazon_bedrock.rs",
        "crates/agena-runtime/src/provider/amazon_bedrock/bedrock_adapter.rs",
    ] {
        let source = fs::read_to_string(workspace.join(relative))
            .with_context(|| format!("read {relative}"))?;
        for forbidden in [
            "aws_smithy_",
            "SdkBody",
            "SmithyEventStreamReceiver",
            "BedrockAnthropicStreamUnmarshaller",
        ] {
            if source.contains(forbidden) {
                bail!("Runtime Bedrock stream consumer must use the SDK leaf: {relative}");
            }
        }
    }
    let adapter_source = fs::read_to_string(
        workspace.join("crates/agena-runtime/src/provider/amazon_bedrock/bedrock_adapter.rs"),
    )
    .context("read Runtime Bedrock streaming adapter")?;
    if !adapter_source.contains("decode_bedrock_anthropic_response(response)") {
        bail!("Runtime Bedrock adapter must invoke the streaming leaf");
    }
    Ok(())
}

fn assert_provider_http_utility_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let app_source = workspace.join("apps/agena/src");
    let utility = workspace.join("crates/agena-provider/src/http_utils.rs");
    let provider_source =
        fs::read_to_string(&utility).with_context(|| format!("read {}", utility.display()))?;
    for required in [
        "pub fn request_shape_fingerprint",
        "pub fn normalize_base_url",
        "pub fn auth_header_value",
        "pub fn merge_json_object_patch_map",
        "pub fn normalize_optional_text",
    ] {
        if !provider_source.contains(required) {
            bail!("provider HTTP utility module must retain `{required}`");
        }
    }
    let core_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/utils.rs"))
            .context("read Core provider utilities")?;
    for forbidden in [
        "fn request_shape_fingerprint",
        "fn normalize_base_url",
        "fn auth_header_value",
        "fn merge_json_object_patch_map",
        "fn normalize_optional_text",
    ] {
        if core_source.contains(forbidden) {
            bail!("Core provider utilities must not retain `{forbidden}`");
        }
    }

    let tui_composer = workspace.join("crates/agena-tui/src/composer.rs");
    let tui_composer_source = fs::read_to_string(&tui_composer)
        .with_context(|| format!("read {}", tui_composer.display()))?;
    for required in [
        "pub struct ComposerItemSelection",
        "pub enum ComposerItemAction",
        "pub enum ComposerItemEffect",
        "pub fn reduce",
    ] {
        if !tui_composer_source.contains(required) {
            bail!("TUI composer interaction slice must retain `{required}`");
        }
    }
    let app_composer = fs::read_to_string(app_source.join("app/app_composer.rs"))
        .context("read final-app composer effect adapter")?;
    if !app_composer.contains(".reduce(action, self.composer_items.len())") {
        bail!("final app must consume the TUI composer-item reducer");
    }
    let app_types = fs::read_to_string(app_source.join("app/app_types.rs"))
        .context("read final-app presentation state")?;
    if app_types.contains("selected_composer_item: Option<usize>")
        || !app_types.contains("composer_item_selection: ComposerItemSelection")
    {
        bail!("final app must not retain a local composer-item selection state");
    }

    let tui_transcript = workspace.join("crates/agena-tui/src/transcript.rs");
    let tui_transcript_source = fs::read_to_string(&tui_transcript)
        .with_context(|| format!("read {}", tui_transcript.display()))?;
    for required in [
        "pub enum TranscriptPointerSelection",
        "pub struct TranscriptTextPosition",
        "pub struct TranscriptTextSelection",
        "pub struct TranscriptPointerGesture",
        "pub fn cell_range_for_line",
        "pub struct TranscriptScrollbarMetrics",
        "pub fn scrollbar_metrics(",
        "pub fn scroll_for_thumb(",
        "pub fn scrollbar_area(",
    ] {
        if !tui_transcript_source.contains(required) {
            bail!("TUI transcript pointer slice must retain `{required}`");
        }
    }
    let app_transcript_types = fs::read_to_string(app_source.join("app/app_types/transcript.rs"))
        .context("read final-app transcript types")?;
    for forbidden in [
        "pub(crate) enum TranscriptPointerSelection",
        "pub(crate) struct TranscriptTextPosition",
        "pub(crate) struct TranscriptTextSelection",
        "pub(crate) struct TranscriptPointerGesture",
    ] {
        if app_transcript_types.contains(forbidden) {
            bail!("final app must consume the TUI transcript pointer value `{forbidden}`");
        }
    }
    let app_transcript_helpers =
        fs::read_to_string(app_source.join("app/app_transcript_helpers.rs"))
            .context("read final-app transcript geometry helpers")?;
    let app_mouse = fs::read_to_string(app_source.join("app/app_mouse.rs"))
        .context("read final-app transcript mouse adapter")?;
    let app_transcript_view = fs::read_to_string(app_source.join("app/view/view_main.rs"))
        .context("read final-app transcript view adapter")?;
    if app_transcript_helpers.contains("struct TranscriptScrollbarMetrics")
        || app_transcript_helpers.contains("fn transcript_scrollbar_metrics")
        || app_transcript_helpers.contains("fn transcript_scroll_for_thumb")
        || app_transcript_helpers.contains("fn transcript_scrollbar_area")
        || !app_mouse.contains("agena_tui::transcript::scrollbar_metrics(")
        || !app_mouse.contains("agena_tui::transcript::scroll_for_thumb(")
        || !app_transcript_view.contains("agena_tui::transcript::scrollbar_area(")
        || !app_transcript_view.contains("agena_tui::transcript::scrollbar_metrics(")
    {
        bail!(
            "final app must adapt TUI transcript scrollbar geometry and pointer-to-scroll policy"
        );
    }

    let tui_flash = workspace.join("crates/agena-tui/src/flash.rs");
    let tui_flash_source =
        fs::read_to_string(&tui_flash).with_context(|| format!("read {}", tui_flash.display()))?;
    for required in [
        "pub const DEFAULT_FLASH_DURATION",
        "pub enum FlashLevel",
        "pub struct FlashMessage",
        "pub fn is_expired_at",
    ] {
        if !tui_flash_source.contains(required) {
            bail!("TUI flash slice must retain `{required}`");
        }
    }
    let app_session_types = fs::read_to_string(app_source.join("app/app_types/session.rs"))
        .context("read final-app session presentation types")?;
    for forbidden in [
        "pub(crate) struct FlashMessage",
        "pub(crate) enum FlashLevel",
    ] {
        if app_session_types.contains(forbidden) {
            bail!("final app must consume the TUI flash value `{forbidden}`");
        }
    }

    let tui_help = workspace.join("crates/agena-tui/src/help.rs");
    let tui_help_source =
        fs::read_to_string(&tui_help).with_context(|| format!("read {}", tui_help.display()))?;
    for required in [
        "pub enum HelpOverlayKind",
        "pub enum ContextHelpPreset",
        "pub fn preset_specs",
        "pub type HelpOverlay",
        "pub fn contextual_document",
        "pub fn plain_text",
    ] {
        if !tui_help_source.contains(required) {
            bail!("TUI help presentation slice must retain `{required}`");
        }
    }
    let app_help = fs::read_to_string(app_source.join("app/app_help.rs"))
        .context("read final-app help adapter")?;
    if app_help.contains("enum InfoOverlayKind")
        || app_help.contains("enum HelpPreset")
        || app_help.contains("type HelpEntrySpec")
        || app_help.contains("type HelpSectionSpec")
        || app_help.contains("fn info_overlay_plain_text")
        || app_help.contains("fn help_preset")
        || !app_help.contains("agena_tui::help::plain_text")
        || !app_help.contains("agena_tui::help::contextual_document")
        || !app_help.contains("agena_tui::help::preset_specs(preset)")
    {
        bail!(
            "final app must consume the TUI help document, overlay identity, and plain-text projection"
        );
    }
    for moved_key in [
        "context-help-summary-sessions",
        "context-help-summary-transcript",
        "context-help-summary-composer",
        "context-help-summary-composer-items",
        "context-help-summary-history",
        "context-help-summary-suggestions",
        "context-help-summary-editor",
        "context-help-summary-editor-multiline",
        "context-help-summary-search-picker",
        "context-help-summary-choice-list",
        "context-help-summary-timeline",
        "context-help-summary-permission",
        "context-help-summary-details",
        "context-help-summary-user-input",
        "context-help-summary-user-input-editor",
        "context-help-summary-user-input-review",
        "context-help-summary-user-input-decision-review",
        "context-help-summary-confirm",
        "context-help-summary-usage",
        "context-help-summary-list",
        "context-help-summary-panes",
        "context-help-summary-action-pane",
        "context-help-summary-provider",
        "context-help-summary-model-catalog",
        "context-help-summary-plugin-list",
        "context-help-summary-plugin-detail",
        "context-help-summary-plugin-config",
        "context-help-summary-plugin-actions",
        "context-help-summary-plugin-selection",
        "context-help-summary-plugin-drilldown",
        "context-help-summary-plugin-diff",
    ] {
        if app_help.contains(moved_key) || !tui_help_source.contains(moved_key) {
            bail!("migrated TUI help-card mapping must have exactly one owner for `{moved_key}`");
        }
    }
    if !app_types.contains("agena_tui::help::{HelpOverlay, HelpOverlayKind}") {
        bail!("final app must not retain a local help-overlay presentation type");
    }

    let tui_main_focus = workspace.join("crates/agena-tui/src/main_focus.rs");
    let tui_main_focus_source = fs::read_to_string(&tui_main_focus)
        .with_context(|| format!("read {}", tui_main_focus.display()))?;
    for required in ["pub enum Focus", "pub fn label", "pub fn move_pane"] {
        if !tui_main_focus_source.contains(required) {
            bail!("TUI main-focus slice must retain `{required}`");
        }
    }
    if app_types.contains("enum Focus")
        || app_types.contains("fn cycle_copy")
        || !app_types.contains("agena_tui::main_focus::Focus")
    {
        bail!("final app must consume the TUI main-focus value and pane policy");
    }
    let app_main_input = fs::read_to_string(app_source.join("app/app_input.rs"))
        .context("read final-app main-focus input adapter")?;
    if !app_main_input.contains(".move_pane(")
        || !app_main_input.contains("agena_tui::main_focus::Focus")
    {
        bail!("final app must use the TUI main-focus pane-navigation policy");
    }

    let tui_session_view = workspace.join("crates/agena-tui/src/session_view.rs");
    let tui_session_view_source = fs::read_to_string(&tui_session_view)
        .with_context(|| format!("read {}", tui_session_view.display()))?;
    for required in ["pub enum SessionViewMode", "pub fn next", "pub fn label"] {
        if !tui_session_view_source.contains(required) {
            bail!("TUI session-view slice must retain `{required}`");
        }
    }
    if app_types.contains("enum SessionViewMode")
        || !app_types.contains("agena_tui::session_view::SessionViewMode")
    {
        bail!("final app must consume the TUI session-view presentation value");
    }
    let app_session_input = fs::read_to_string(app_source.join("app/app_session_input.rs"))
        .context("read final-app session-view input adapter")?;
    let app_session_commands = fs::read_to_string(app_source.join("app/app_command_actions.rs"))
        .context("read final-app session-view command adapter")?;
    if !app_session_input.contains("agena_tui::session_view::SessionViewMode")
        || !app_session_commands.contains("agena_tui::session_view::SessionViewMode")
        || !app_session_commands.contains("self.sessions.view_mode().next()")
    {
        bail!("final app must adapt the TUI session-view selector instead of owning it");
    }

    let tui_exports = fs::read_to_string(workspace.join("crates/agena-tui/src/lib.rs"))
        .context("read TUI module exports")?;
    let tui_session_search = workspace.join("crates/agena-tui/src/session_search.rs");
    let tui_session_search_source = fs::read_to_string(&tui_session_search)
        .with_context(|| format!("read {}", tui_session_search.display()))?;
    for required in [
        "pub struct SessionSearchItem",
        "pub enum SessionSearchEffect",
        "pub struct SessionSearchPresentation",
        "pub fn matches_query",
        "pub fn reset_for_query",
        "pub fn request_next_page",
        "pub fn apply_page",
        "pub fn reject_page",
        "pub type SessionSearchOverlay",
        "pub fn render_overlay",
    ] {
        if !tui_session_search_source.contains(required) {
            bail!("TUI session-search slice must retain `{required}`");
        }
    }
    if app_source.join("app/app_search_items.rs").exists()
        || app_source.join("app/app_choice_custom_value.rs").exists()
    {
        bail!(
            "obsolete final-app generic search-item and choice-custom modules must remain deleted"
        );
    }
    let tui_command_palette = workspace.join("crates/agena-tui/src/command_palette.rs");
    let tui_command_palette_source = fs::read_to_string(&tui_command_palette)
        .with_context(|| format!("read {}", tui_command_palette.display()))?;
    if !tui_exports.contains("pub mod command_palette;") {
        bail!("TUI command-palette presentation module must remain exported");
    }
    for required in [
        "pub struct CommandPaletteItem",
        "impl SearchPickerItem for CommandPaletteItem",
        "pub type CommandPalettePresentation",
        "pub enum CommandPaletteAction",
        "pub enum CommandPaletteEffect",
        "pub fn new_presentation",
        "pub fn reduce",
    ] {
        if !tui_command_palette_source.contains(required) {
            bail!("TUI command-palette slice must retain `{required}`");
        }
    }
    let app_session_types = fs::read_to_string(app_source.join("app/app_types/session.rs"))
        .context("read final-app command-palette effect types")?;
    let app_command_palette =
        fs::read_to_string(app_source.join("app/app_settings_choices/navigation.rs"))
            .context("read final-app command-palette builder")?;
    let app_command_palette_input = fs::read_to_string(app_source.join("app/app_overlays.rs"))
        .context("read final-app command-palette effect adapter")?;
    let app_picker_views =
        fs::read_to_string(app_source.join("app/view/view_overlays/overlay_core.rs"))
            .context("read final-app picker rendering adapter")?;
    if !app_session_types.contains("struct CommandPaletteOverlay")
        || !app_session_types
            .contains("presentation: agena_tui::command_palette::CommandPalettePresentation")
        || !app_session_types.contains("actions: BTreeMap<String, CommandPaletteCommand>")
        || app_session_types.contains("PickerValue::Command")
        || app_session_types.contains("PickerValue::PluginCommand")
        || app_session_types.contains("PickerKind::Commands")
        || !app_command_palette.contains("agena_tui::command_palette::new_presentation")
        || !app_command_palette.contains("Route::CommandPalette")
        || !app_command_palette_input.contains("handle_command_palette_key")
        || !app_command_palette_input.contains("agena_tui::command_palette::reduce")
        || !tui_session_search_source.contains("pub fn render_overlay")
        || !app_picker_views.contains("agena_tui::session_search::render_overlay(")
        || !tui_command_palette_source.contains("pub fn render_overlay")
        || !app_picker_views.contains("agena_tui::command_palette::render_overlay(")
        || app_picker_views.contains("fn render_command_palette_overlay(")
    {
        bail!(
            "command palette must keep display/search reduction in TUI and concrete command effects in App"
        );
    }
    let tui_session_navigation = workspace.join("crates/agena-tui/src/session_navigation.rs");
    let tui_session_navigation_source = fs::read_to_string(&tui_session_navigation)
        .with_context(|| format!("read {}", tui_session_navigation.display()))?;
    if !tui_exports.contains("pub mod session_navigation;") {
        bail!("TUI session-navigation presentation module must remain exported");
    }
    for required in [
        "pub struct SessionNavigationItem",
        "impl SearchPickerItem for SessionNavigationItem",
        "pub enum SessionNavigationMode",
        "pub type SessionNavigationPresentation",
        "pub enum SessionNavigationAction",
        "pub enum SessionNavigationEffect",
        "pub struct SessionLineageNode",
        "pub enum SessionLineageRelation",
        "pub struct SessionLineageItem",
        "pub struct SessionLineageSummary",
        "pub fn build_lineage_items",
        "pub fn summarize_lineage_items",
        "pub fn new_presentation",
        "pub fn reduce",
        "pub fn render_overlay",
    ] {
        if !tui_session_navigation_source.contains(required) {
            bail!("TUI session-navigation slice must retain `{required}`");
        }
    }
    let app_session_navigation = fs::read_to_string(app_source.join("app/app_navigation.rs"))
        .context("read final-app session-navigation projection")?;
    let app_session_navigation_events =
        fs::read_to_string(app_source.join("app/app_session_events/handlers.rs"))
            .context("read final-app session-navigation result adapter")?;
    if !app_session_types.contains("struct SessionNavigationOverlay")
        || !app_session_types
            .contains("presentation: agena_tui::session_navigation::SessionNavigationPresentation")
        || !app_session_types.contains("actions: BTreeMap<String, SessionNavigationCommand>")
        || app_session_types.contains("Session(i64)")
        || app_session_types.contains("Message(Box<MessageResource>)")
        || !app_session_navigation.contains("build_session_navigation_overlay")
        || !app_session_navigation.contains("SessionNavigationCommand::OpenSession")
        || !app_session_navigation.contains("SessionNavigationCommand::Rewind")
        || !app_session_navigation_events.contains("take_session_navigation_route")
        || !app_session_navigation_events.contains("SessionNavigationQuery::Lineage")
        || !app_session_navigation_events.contains("SessionNavigationQuery::RewindMessages")
        || !app_session_navigation_events.contains("SessionNavigationQuery::ChildSessions")
        || !app_session_navigation_events.contains("SessionLineageNode")
        || !app_session_navigation_events.contains("build_lineage_items")
        || !app_session_navigation_events.contains("summarize_lineage_items")
        || app_session_types.contains("enum LineageRelation")
        || app_session_types.contains("struct LineageSessionItem")
        || app_session_types.contains("struct SessionLineageSummary")
        || !app_command_palette_input.contains("handle_session_navigation_key")
        || !app_command_palette_input.contains("agena_tui::session_navigation::reduce")
        || !app_picker_views.contains("agena_tui::session_navigation::render_overlay(")
        || app_picker_views.contains("fn render_session_navigation_overlay(")
    {
        bail!(
            "session navigation must keep lineage/display/search reduction in TUI and concrete session effects in App"
        );
    }
    let tui_selection_picker = workspace.join("crates/agena-tui/src/selection_picker.rs");
    let tui_selection_picker_source = fs::read_to_string(&tui_selection_picker)
        .with_context(|| format!("read {}", tui_selection_picker.display()))?;
    if !tui_exports.contains("pub mod selection_picker;") {
        bail!("TUI selection-picker presentation module must remain exported");
    }
    for required in [
        "pub struct SelectionPickerItem",
        "impl SearchPickerItem for SelectionPickerItem",
        "pub type SelectionPickerPresentation",
        "pub enum SelectionPickerAction",
        "pub enum SelectionPickerEffect",
        "pub fn new_presentation",
        "pub fn reduce",
        "pub fn render_overlay",
    ] {
        if !tui_selection_picker_source.contains(required) {
            bail!("TUI selection-picker slice must retain `{required}`");
        }
    }
    let app_provider_picker =
        fs::read_to_string(app_source.join("app/app_provider_runtime/catalog.rs"))
            .context("read final-app selection-picker projection")?;
    if !app_session_types.contains("struct SelectionPickerOverlay")
        || !app_session_types
            .contains("presentation: agena_tui::selection_picker::SelectionPickerPresentation")
        || !app_session_types.contains("actions: BTreeMap<String, SelectionPickerCommand>")
        || app_session_types.contains("struct PickerItem")
        || app_session_types.contains("enum PickerValue")
        || app_session_types.contains("enum PickerKind")
        || !app_provider_picker.contains("SelectionPickerQuery::Providers")
        || !app_provider_picker.contains("SelectionPickerQuery::Agents")
        || !app_provider_picker.contains("SelectionPickerQuery::SessionAgents")
        || !app_command_palette_input.contains("handle_selection_picker_key")
        || !app_command_palette_input.contains("agena_tui::selection_picker::reduce")
        || !app_picker_views.contains("agena_tui::selection_picker::render_overlay(")
        || app_picker_views.contains("fn render_selection_picker_overlay(")
    {
        bail!(
            "generic selection picker must keep display/search reduction in TUI and concrete actions in App"
        );
    }
    let runtime_builder =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/builder.rs"))
            .context("read Runtime concrete builder visibility")?;
    if !runtime_builder.contains("pub(crate) fn current_snapshot(&self) -> Arc<RuntimeSnapshot>")
        || runtime_builder.contains("pub fn current_snapshot(&self) -> Arc<RuntimeSnapshot>")
    {
        bail!(
            "Runtime concrete snapshot handle must remain crate-private behind service capabilities"
        );
    }
    let runtime_tool_registry =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/tool/tool_registry.rs"))
            .context("read Runtime concrete tool-executor visibility")?;
    let runtime_session_manager =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/session/manager/mod.rs"))
            .context("read Runtime session-manager tool boundary")?;
    if !runtime_tool_registry.contains("pub(crate) struct ToolExecutor")
        || runtime_tool_registry.contains("pub struct ToolExecutor")
        || !runtime_session_manager.contains("pub(crate) fn tool_executor(&self) -> ToolExecutor")
        || runtime_session_manager.contains("pub fn tool_executor(&self) -> ToolExecutor")
    {
        bail!(
            "Runtime concrete tool executor must remain crate-private behind tool execution services"
        );
    }
    if !runtime_session_manager
        .contains("pub(crate) fn event_publisher(&self) -> Arc<crate::event::EventPublisher>")
        || runtime_session_manager
            .contains("pub fn event_publisher(&self) -> Arc<crate::event::EventPublisher>")
    {
        bail!("Runtime concrete event publisher must remain crate-private behind event services");
    }
    let runtime_provider_registry =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/registry/mod.rs"))
            .context("read Runtime concrete provider-registry visibility")?;
    if !runtime_provider_registry.contains("pub(crate) struct ProviderRegistry")
        || runtime_provider_registry.contains("pub struct ProviderRegistry")
    {
        bail!(
            "Runtime concrete provider registry must remain crate-private behind provider services"
        );
    }
    let runtime_snapshot =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/snapshot/mod.rs"))
            .context("read Runtime concrete snapshot accessor visibility")?;
    for required in [
        "pub(crate) fn provider_registry(&self) -> Arc<ProviderRegistry>",
        "pub(crate) fn catalog_source_provider_registry(&self) -> Arc<ProviderRegistry>",
        "pub(crate) fn model_catalog(&self) -> Arc<ModelCatalogService>",
        "pub(crate) fn mcp_manager(&self) -> Option<Arc<agena_mcp_client::McpConnectionManager>>",
        "pub(crate) fn plugin_manager(&self) -> Arc<PluginHost>",
    ] {
        if !runtime_snapshot.contains(required) {
            bail!("Runtime snapshot concrete accessor must remain crate-private: `{required}`");
        }
    }
    for forbidden in [
        "pub fn provider_registry(&self) -> Arc<ProviderRegistry>",
        "pub fn catalog_source_provider_registry(&self) -> Arc<ProviderRegistry>",
        "pub fn model_catalog(&self) -> Arc<ModelCatalogService>",
        "pub fn mcp_manager(&self) -> Option<Arc<agena_mcp_client::McpConnectionManager>>",
        "pub fn plugin_manager(&self) -> Arc<PluginHost>",
    ] {
        if runtime_snapshot.contains(forbidden) {
            bail!("Runtime snapshot must not restore public concrete accessor `{forbidden}`");
        }
    }
    if app_session_types.contains("struct SessionSearchItem")
        || app_session_types.contains("struct SessionSearchOverlayMeta")
        || app_session_types.contains("type SessionSearchOverlay =")
    {
        bail!("final app must not retain the session-search presentation state");
    }
    let app_session_overlays = fs::read_to_string(app_source.join("app/app_overlays.rs"))
        .context("read final-app session-search effect adapter")?;
    let app_session_events =
        fs::read_to_string(app_source.join("app/app_session_events/handlers.rs"))
            .context("read final-app session-search result adapter")?;
    if app_session_overlays.contains("session_matches_query(")
        || !app_session_overlays.contains("dialog.meta.request_next_page()")
        || !app_session_overlays.contains("dialog.meta.reset_for_query()")
        || !app_session_events.contains(".apply_page(")
        || !app_session_events.contains(".reject_page(")
    {
        bail!("final app must adapt TUI session-search state transitions instead of owning them");
    }

    let tui_file_attach = workspace.join("crates/agena-tui/src/file_attach.rs");
    let tui_file_attach_source = fs::read_to_string(&tui_file_attach)
        .with_context(|| format!("read {}", tui_file_attach.display()))?;
    if !tui_exports.contains("pub mod file_attach;") {
        bail!("TUI file-attach presentation module must remain exported");
    }
    for required in [
        "pub struct FileAttachItem",
        "pub struct FileAttachCustomValue",
        "pub type FileAttachPresentation",
        "pub enum FileAttachEffect",
        "pub fn handle_key",
        "pub fn render_overlay",
    ] {
        if !tui_file_attach_source.contains(required) {
            bail!("TUI file-attach slice must retain `{required}`");
        }
    }
    let app_file_attach_input = fs::read_to_string(app_source.join("app/app_overlays.rs"))
        .context("read final-app file-attach effect adapter")?;
    let app_file_attach_state = fs::read_to_string(app_source.join("app/app_composer_state.rs"))
        .context("read final-app file-attach filesystem adapter")?;
    let app_file_attach_types = fs::read_to_string(app_source.join("app/app_types/overlays.rs"))
        .context("read final-app file-attach overlay state")?;
    let app_overlay_core =
        fs::read_to_string(app_source.join("app/view/view_overlays/overlay_core.rs"))
            .context("read final-app overlay renderer adapter")?;
    if app_file_attach_types.contains("struct FileAttachOverlayMeta")
        || app_file_attach_types.contains("struct TypedPathValue")
        || app_file_attach_types.contains("SearchPicker<PathBuf, TypedPathValue")
        || !app_file_attach_types.contains("presentation: FileAttachPresentation")
        || !app_file_attach_types.contains("path_actions: BTreeMap<String, PathBuf>")
        || !app_file_attach_input
            .contains("agena_tui::file_attach::handle_key(&mut dialog.presentation, key)")
        || !app_file_attach_state.contains("agena_tui::file_attach::FileAttachItem")
        || !app_overlay_core.contains("agena_tui::file_attach::render_overlay(")
        || app_overlay_core.contains("fn render_file_attach_overlay(")
    {
        bail!(
            "final app must adapt the TUI file-attach reducer while retaining filesystem effects"
        );
    }

    let tui_plugin_workbench = workspace.join("crates/agena-tui/src/plugin_workbench.rs");
    let tui_plugin_workbench_source = fs::read_to_string(&tui_plugin_workbench)
        .with_context(|| format!("read {}", tui_plugin_workbench.display()))?;
    if !tui_exports.contains("pub mod plugin_workbench;") {
        bail!("TUI plugin-workbench navigation module must remain exported");
    }
    for required in [
        "pub enum PluginWorkbenchMode",
        "pub enum PluginDetailTab",
        "pub enum PluginTransportFilter",
        "pub enum PluginConfigFilter",
        "pub struct PluginWorkbenchListItem",
        "pub struct PluginWorkbenchListPresentation",
        "pub enum PluginWorkbenchListEffect",
        "pub fn handle_list_key",
        "pub struct PluginConfigPickerItem",
        "pub type PluginConfigPickerPresentation",
        "pub enum PluginConfigPickerAction",
        "pub enum PluginConfigPickerEffect",
        "pub fn new_plugin_config_picker",
        "pub fn reduce_plugin_config_picker",
        "pub struct PluginWorkbenchNavigation",
        "pub enum PluginWorkbenchNavigationEffect",
        "pub fn handle_key",
    ] {
        if !tui_plugin_workbench_source.contains(required) {
            bail!("TUI plugin-workbench navigation slice must retain `{required}`");
        }
    }
    let app_plugin_workbench = fs::read_to_string(app_source.join("app/plugin_workbench.rs"))
        .context("read final-app plugin-workbench state")?;
    let app_test_fixtures = fs::read_to_string(app_source.join("app/app_tests.rs"))
        .context("read final-app transcript fixture boundary")?;
    let app_transcript_fixtures = fs::read_to_string(app_source.join("app/transcript_view.rs"))
        .context("read final-app transcript renderer fixture boundary")?;
    let app_root = fs::read_to_string(workspace.join("apps/agena/src/app.rs"))
        .context("read final-app transcript fixture builders")?;
    if app_test_fixtures.contains("agena_runtime::message")
        || app_test_fixtures.contains("agena_runtime::session::project_message_part")
        || app_transcript_fixtures.contains("agena_runtime::message")
        || app_transcript_fixtures.contains("agena_runtime::session")
        || app_root.contains("pub(crate) use agena_runtime::message::PartContent")
    {
        bail!(
            "final-app text/reasoning transcript fixtures must construct public API resources without Runtime message parts"
        );
    }
    let app_plugin_input =
        fs::read_to_string(app_source.join("app/plugin_workbench/workbench_input.rs"))
            .context("read final-app plugin-workbench navigation adapter")?;
    let app_plugin_config =
        fs::read_to_string(app_source.join("app/plugin_workbench/workbench_config.rs"))
            .context("read final-app plugin-config action projection")?;
    let app_plugin_selection =
        fs::read_to_string(app_source.join("app/plugin_workbench/workbench_navigation.rs"))
            .context("read final-app plugin-config selection projection")?;
    if app_plugin_workbench.contains("enum PluginWorkbenchMode")
        || app_plugin_workbench.contains("enum PluginDetailTab")
        || app_plugin_workbench.contains("enum PluginTransportFilter")
        || app_plugin_workbench.contains("enum PluginConfigFilter")
        || !app_plugin_workbench.contains("navigation: PluginWorkbenchNavigation")
        || !app_plugin_workbench.contains("list: PluginWorkbenchListPresentation")
        || app_plugin_workbench.contains("visible_plugins:")
        || app_plugin_workbench.contains("selected_plugin:")
        || !app_plugin_input.contains("handle_plugin_workbench_navigation_key")
        || !app_plugin_input.contains("PluginWorkbenchNavigationEffect::OpenSelected")
        || !app_plugin_input.contains("PluginWorkbenchNavigationEffect::ScrollDetail")
        || app_plugin_workbench.contains("type PluginConfigActionOverlay =")
        || app_plugin_workbench.contains("type PluginConfigSelectionOverlay =")
        || app_plugin_workbench.contains("struct PluginConfigSelectionMeta")
        || app_plugin_workbench.contains("struct PluginConfigActionItem")
        || app_plugin_workbench.contains("struct PluginConfigSelectionItem")
        || app_plugin_workbench
            .contains("impl agena_tui_components::SearchPickerItem for PluginConfig")
        || !app_plugin_config.contains("new_plugin_config_picker")
        || !app_plugin_selection.contains("new_plugin_config_picker")
        || !app_plugin_input.contains("reduce_plugin_config_picker")
    {
        bail!(
            "final app must adapt TUI plugin-workbench navigation while retaining configuration effects"
        );
    }

    let tui_session_list = workspace.join("crates/agena-tui/src/session_list.rs");
    let tui_session_list_source = fs::read_to_string(&tui_session_list)
        .with_context(|| format!("read {}", tui_session_list.display()))?;
    for required in [
        "pub struct SessionListItem",
        "pub enum SessionListAction",
        "pub enum SessionListEffect",
        "pub struct SessionListView",
        "pub struct SessionListPresentation",
        "pub fn replace_items",
        "pub fn update",
        "pub fn view",
    ] {
        if !tui_session_list_source.contains(required) {
            bail!("TUI session-list slice must retain `{required}`");
        }
    }
    let app_session_helpers = fs::read_to_string(app_source.join("app/app_session_helpers.rs"))
        .context("read final-app session-list helpers")?;
    let app_session_dispatch =
        fs::read_to_string(app_source.join("app/app_session_events/dispatch.rs"))
            .context("read final-app session-list result adapter")?;
    let app_navigation = fs::read_to_string(app_source.join("app/app_navigation.rs"))
        .context("read final-app session-list view adapter")?;
    if app_session_types.contains("struct SessionListState")
        || app_session_helpers.contains("fn build_visible_session_items")
        || app_session_helpers.contains("fn append_session_subtree")
        || app_session_helpers.contains("fn session_matches_query")
        || app_session_helpers.contains("fn session_sort_recent")
        || !app_types.contains("agena_tui::session_list::SessionListPresentation")
        || !app_session_types.contains("struct SessionListLoadState")
        || !app_session_input.contains("SessionListAction::OpenSelected")
        || !app_session_dispatch.contains(".replace_items(")
        || !app_session_dispatch.contains("SessionListItem {")
        || !app_navigation.contains("self.sessions.view()")
    {
        bail!(
            "final app must adapt the TUI session-list reducer and keep only Runtime load lifecycle"
        );
    }

    let tui_model_chooser = workspace.join("crates/agena-tui/src/model_chooser.rs");
    let tui_model_chooser_source = fs::read_to_string(&tui_model_chooser)
        .with_context(|| format!("read {}", tui_model_chooser.display()))?;
    for required in [
        "pub struct SessionModelIdentity",
        "pub enum SessionModelChooserPurpose",
        "pub struct SessionModelChoiceItem",
        "pub struct SessionModelChooserPresentation",
        "pub enum SessionModelChooserEffect",
        "pub fn selection_effect",
        "pub type SessionModelChooserOverlay",
        "pub fn new_presentation",
        "pub enum SessionModelChooserAction",
        "pub enum SessionModelChooserReducerEffect",
        "pub fn reduce",
        "pub fn refresh",
        "pub fn render_overlay",
    ] {
        if !tui_model_chooser_source.contains(required) {
            bail!("TUI session-model chooser slice must retain `{required}`");
        }
    }
    let app_model_overlays = fs::read_to_string(app_source.join("app/app_overlays.rs"))
        .context("read final-app session-model effect adapter")?;
    let app_model_build =
        fs::read_to_string(app_source.join("app/app_session_interactive/overlays.rs"))
            .context("read final-app session-model picker builder")?;
    let app_model_view =
        fs::read_to_string(app_source.join("app/view/view_overlays/overlay_core.rs"))
            .context("read final-app session-model picker renderer")?;
    let app_model_navigation = fs::read_to_string(app_source.join("app/app_navigation.rs"))
        .context("read final-app session-model picker navigation adapter")?;
    if app_session_types.contains("struct SessionModelChooserOverlayMeta")
        || app_session_types.contains("enum SessionModelChooserPurpose")
        || app_session_types.contains("struct SessionModelChoiceItem")
        || app_session_types.contains("type SessionModelChooserOverlay")
        || app_model_overlays.contains("dialog.meta.selection_effect(&item)")
        || !app_model_overlays.contains("model_ref_from_session_model_identity")
        || !app_model_overlays.contains("agena_tui::model_chooser::reduce(")
        || !app_model_build.contains("agena_tui::model_chooser::new_presentation(")
        || !app_model_view.contains("agena_tui::model_chooser::render_overlay(")
        || app_model_view.contains("fn render_session_model_chooser_overlay(")
        || app_model_navigation.contains("refresh_session_model_chooser_overlay")
    {
        bail!("final app must adapt TUI session-model picker state and selection intent");
    }

    let tui_choice = workspace.join("crates/agena-tui/src/choice.rs");
    let tui_choice_source = fs::read_to_string(&tui_choice)
        .with_context(|| format!("read {}", tui_choice.display()))?;
    for required in [
        "pub struct ChoicePickerItem",
        "impl SearchPickerItem for ChoicePickerItem",
        "pub struct ChoicePresentationMeta",
        "pub struct ChoiceCustomValue",
        "pub type ChoicePresentation",
        "pub enum ChoicePresentationAction",
        "pub enum ChoiceSelection",
        "pub enum ChoicePresentationEffect",
        "pub fn new_presentation",
        "pub fn reduce",
        "pub fn refresh",
        "pub fn sync_input",
        "pub fn select_current",
        "pub fn select_query_row",
        "pub fn render_overlay",
    ] {
        if !tui_choice_source.contains(required) {
            bail!("TUI choice-picker row must retain `{required}`");
        }
    }
    let app_overlay_types = fs::read_to_string(app_source.join("app/app_types/overlays.rs"))
        .context("read final-app choice-picker presentation types")?;
    let app_choice_input = fs::read_to_string(app_source.join("app/app_overlays.rs"))
        .context("read final-app choice effect adapter")?;
    let app_choice_build =
        fs::read_to_string(app_source.join("app/app_session_interactive/overlays.rs"))
            .context("read final-app choice presentation builder")?;
    if app_overlay_types.contains("struct ChoiceItem")
        || app_overlay_types.contains("struct ChoiceOverlayMeta")
        || app_overlay_types.contains("struct ChoiceCustomValue")
        || app_overlay_types.contains("type ChoiceOverlay =")
        || app_overlay_types.contains("enum ChoiceOverlayStyle")
        || !app_overlay_types.contains("agena_tui::choice::ChoicePickerItem as ChoiceItem")
        || !app_overlay_types.contains("presentation: agena_tui::choice::ChoicePresentation")
        || !app_choice_build.contains("agena_tui::choice::new_presentation")
        || !app_choice_input.contains("agena_tui::choice::reduce")
        || !app_choice_input.contains("ChoicePresentationEffect::Commit")
        || app_choice_input.contains("sync_choice_overlay_input")
        || app_choice_input.contains("refresh_choice_overlay")
        || app_choice_input.contains("select_current_choice_overlay_row")
        || app_choice_input.contains("select_choice_overlay_query_row")
        || !app_picker_views.contains("agena_tui::choice::render_overlay(")
        || app_picker_views.contains("fn render_choice_overlay(")
    {
        bail!(
            "final app must adapt the complete TUI choice presentation and keep only concrete actions"
        );
    }

    let tui_timeline = workspace.join("crates/agena-tui/src/timeline.rs");
    let tui_timeline_source = fs::read_to_string(&tui_timeline)
        .with_context(|| format!("read {}", tui_timeline.display()))?;
    for required in [
        "pub struct TimelineItem",
        "pub struct TimelinePresentation",
        "pub enum TimelineEffect",
        "pub fn selection_effect",
        "pub type TimelineOverlay",
        "pub fn render_overlay",
    ] {
        if !tui_timeline_source.contains(required) {
            bail!("TUI timeline picker slice must retain `{required}`");
        }
    }
    let app_timeline_input = fs::read_to_string(app_source.join("app/app_overlays.rs"))
        .context("read final-app timeline effect adapter")?;
    let app_timeline_builder =
        fs::read_to_string(app_source.join("app/app_session_interactive/overlays.rs"))
            .context("read final-app timeline picker builder")?;
    let app_timeline_view =
        fs::read_to_string(app_source.join("app/view/view_overlays/overlay_core.rs"))
            .context("read final-app timeline picker renderer adapter")?;
    if app_overlay_types.contains("struct TimelineOverlayMeta")
        || app_overlay_types.contains("struct TimelineItem")
        || app_overlay_types.contains("type TimelineOverlay")
        || !app_timeline_input.contains("dialog.meta.selection_effect(item)")
        || !app_timeline_builder.contains("TimelinePresentation::new(session_id)")
        || !app_timeline_view
            .contains("agena_tui::timeline::render_overlay(frame, area, dialog, &self.i18n)")
        || app_timeline_view.contains("fn render_timeline_overlay")
    {
        bail!(
            "final app must adapt the complete TUI timeline presentation, view, and selection intent"
        );
    }

    let tui_prompt_history = workspace.join("crates/agena-tui/src/prompt_history.rs");
    let tui_prompt_history_source = fs::read_to_string(&tui_prompt_history)
        .with_context(|| format!("read {}", tui_prompt_history.display()))?;
    for required in [
        "pub struct PromptHistorySearchResult",
        "pub type PromptHistorySearchState",
        "pub enum PromptHistoryPickerEffect",
        "pub fn handle_key",
        "pub fn render_overlay",
    ] {
        if !tui_prompt_history_source.contains(required) {
            bail!("TUI prompt-history picker slice must retain `{required}`");
        }
    }
    let app_composer_types = fs::read_to_string(app_source.join("app/app_types/composer.rs"))
        .context("read final-app composer presentation types")?;
    let app_composer_input = fs::read_to_string(app_source.join("app/app_composer.rs"))
        .context("read final-app prompt-history effect adapter")?;
    let app_composer_view = fs::read_to_string(app_source.join("app/view/view_main.rs"))
        .context("read final-app composer picker rendering adapter")?;
    if app_composer_types.contains("struct PromptHistorySearchResult")
        || app_composer_types.contains("type PromptHistorySearchState")
        || app_composer_types.contains("impl SearchPickerItem for PromptHistorySearchResult")
        || app_composer_input.contains("enum PromptHistoryPickerOutcome")
        || app_composer_input.contains("fn handle_prompt_history_picker_key")
        || !app_composer_input.contains("agena_tui::prompt_history::handle_key")
        || !app_composer_input.contains("PromptHistoryPickerEffect::UseText")
        || !app_composer_view.contains("agena_tui::prompt_history::render_overlay(")
        || app_composer_view.contains("fn render_prompt_history_picker(")
    {
        bail!("final app must adapt the TUI prompt-history picker reducer");
    }

    let tui_slash_commands = workspace.join("crates/agena-tui/src/slash_commands.rs");
    let tui_slash_commands_source = fs::read_to_string(&tui_slash_commands)
        .with_context(|| format!("read {}", tui_slash_commands.display()))?;
    for required in [
        "pub struct SlashCommandSuggestionMeta",
        "pub struct SlashCommandSuggestionItem",
        "pub type SlashCommandSuggestionState",
        "pub enum SlashCommandSuggestionEffect",
        "pub fn handle_key",
        "SlashCommandSuggestionEffect::Fill",
        "SlashCommandSuggestionEffect::Accept",
        "pub fn render_overlay",
    ] {
        if !tui_slash_commands_source.contains(required) {
            bail!("TUI slash-command suggestion slice must retain `{required}`");
        }
    }
    let app_state_types = fs::read_to_string(app_source.join("app/app_types.rs"))
        .context("read final-app slash-command state boundary")?;
    if app_composer_types.contains("struct SlashCommandSuggestionMeta")
        || app_composer_types.contains("struct SlashCommandSuggestionItem")
        || app_composer_types.contains("type SlashCommandSuggestionState")
        || app_composer_types.contains("enum SlashCommandSuggestionValue")
        || app_composer_types.contains("impl SearchPickerItem for SlashCommandSuggestionItem")
        || app_composer_types.contains("CommandSpec")
        || app_composer_types.contains("PluginCommandCatalogItem")
        || !app_composer_types.contains("struct SlashCommandSuggestionAction")
        || !app_state_types.contains(
            "slash_command_suggestion_actions: BTreeMap<String, SlashCommandSuggestionAction>",
        )
        || !app_composer_input.contains("agena_tui::slash_commands::handle_key")
        || !app_composer_input.contains("complete_slash_command_suggestion(key, false)")
        || !app_composer_input.contains("complete_slash_command_suggestion(key, true)")
        || !app_composer_view.contains("agena_tui::slash_commands::render_overlay(")
        || app_composer_view.contains("fn render_slash_command_picker(")
    {
        bail!(
            "final app must adapt TUI slash-command picker intents through a separate action map"
        );
    }

    let tui_file_mentions = workspace.join("crates/agena-tui/src/file_mentions.rs");
    let tui_file_mentions_source = fs::read_to_string(&tui_file_mentions)
        .with_context(|| format!("read {}", tui_file_mentions.display()))?;
    for required in [
        "pub struct FileMentionSuggestionMeta",
        "pub struct FileMentionSuggestionItem",
        "pub type FileMentionSuggestionState",
        "pub enum FileMentionSuggestionEffect",
        "pub fn new_state",
        "pub fn handle_key",
        "FileMentionSuggestionEffect::Refresh",
        "FileMentionSuggestionEffect::Select",
        "pub fn render_overlay",
    ] {
        if !tui_file_mentions_source.contains(required) {
            bail!("TUI file-mention suggestion slice must retain `{required}`");
        }
    }
    if app_composer_types.contains("struct FileMentionSuggestionMeta")
        || app_composer_types.contains("struct FileMentionSuggestionItem")
        || app_composer_types.contains("type FileMentionSuggestionState")
        || app_composer_types.contains("impl SearchPickerItem for FileMentionSuggestionItem")
        || !app_composer_types.contains("struct FileMentionSuggestionAction")
        || !app_state_types.contains(
            "file_mention_suggestion_actions: BTreeMap<String, FileMentionSuggestionAction>",
        )
        || !app_composer_input.contains("agena_tui::file_mentions::handle_key")
        || !app_composer_input.contains("FileMentionSuggestionEffect::Refresh")
        || !app_composer_input.contains("complete_file_mention_suggestion(key)")
        || !app_composer_view.contains("agena_tui::file_mentions::render_overlay(")
        || app_composer_view.contains("fn render_file_mention_picker(")
    {
        bail!(
            "final app must adapt TUI file-mention picker intents through a separate path-action map"
        );
    }

    let tui_path_browser = workspace.join("crates/agena-tui/src/path_browser.rs");
    let tui_path_browser_source = fs::read_to_string(&tui_path_browser)
        .with_context(|| format!("read {}", tui_path_browser.display()))?;
    for required in [
        "pub enum PathBrowserMode",
        "pub struct PathBrowserItem",
        "pub type PathBrowserPresentation",
        "pub enum PathBrowserEffect",
        "pub fn new_presentation",
        "pub fn handle_key",
        "pub fn render_overlay",
    ] {
        if !tui_path_browser_source.contains(required) {
            bail!("TUI path-browser slice must retain `{required}`");
        }
    }
    let app_path_browser =
        fs::read_to_string(app_source.join("app/app_permissions/path_browser.rs"))
            .context("read final-app path-browser effect adapter")?;
    if app_overlay_types.contains("struct PathBrowserOverlayMeta")
        || app_overlay_types.contains("struct PathBrowserItem")
        || !app_overlay_types
            .contains("presentation: agena_tui::path_browser::PathBrowserPresentation")
        || !app_overlay_types.contains("path_actions: BTreeMap<String, PathBuf>")
        || !app_path_browser.contains("agena_tui::path_browser::handle_key")
        || !app_path_browser.contains("dialog.path_actions")
        || !app_overlay_core.contains("agena_tui::path_browser::render_overlay(")
        || app_overlay_core.contains("fn render_path_browser_overlay(")
    {
        bail!("final app must adapt the TUI path-browser reducer through a path action map");
    }

    let tui_settings_studio = workspace.join("crates/agena-tui/src/settings_studio.rs");
    let tui_settings_studio_source = fs::read_to_string(&tui_settings_studio)
        .with_context(|| format!("read {}", tui_settings_studio.display()))?;
    for required in [
        "pub enum SettingsStudioSectionId",
        "pub struct SettingsStudioSourceRow",
        "pub struct SettingsStudioItem<A>",
        "pub struct SettingsStudioSection<A>",
        "pub struct SettingsStudioPresentation<A>",
        "pub enum SettingsStudioEffect",
        "pub fn section_group_label",
        "pub fn select_query",
        "pub fn handle_key",
    ] {
        if !tui_settings_studio_source.contains(required) {
            bail!("TUI settings-studio slice must retain `{required}`");
        }
    }
    let app_settings_input =
        fs::read_to_string(app_source.join("app/app_permissions/overlay_handlers.rs"))
            .context("read final-app settings-studio input adapter")?;
    let app_settings_selection =
        fs::read_to_string(app_source.join("app/app_permission_studio.rs"))
            .context("read final-app settings-studio selection adapter")?;
    let app_settings_builder =
        fs::read_to_string(app_source.join("app/app_session_interactive/settings.rs"))
            .context("read final-app settings-studio builder")?;
    let app_studio_impls = fs::read_to_string(app_source.join("app/app_studio_state_impls.rs"))
        .context("read final-app studio presentation implementations")?;
    let app_settings_view =
        fs::read_to_string(app_source.join("app/view/view_settings_helpers.rs"))
            .context("read final-app settings-studio view helpers")?;
    for forbidden in [
        "struct SettingsStudioSection",
        "struct SettingsStudioItem",
        "struct SettingsSourceRow",
        "type SettingsStudioFocus =",
        "enum SettingsStudioSectionId",
        "impl SettingsStudioItem",
        "impl SettingsSourceRow",
        "impl SectionedListSection for SettingsStudioSection",
        "fn settings_section_group_label",
    ] {
        if app_overlay_types.contains(forbidden) || app_studio_impls.contains(forbidden) {
            bail!("final app must not restore local settings-studio presentation `{forbidden}`");
        }
    }
    let runtime_root = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
        .context("read Runtime module visibility")?;
    let runtime_session_module =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/session/mod.rs"))
            .context("read Runtime session projection visibility")?;
    let runtime_session_manager =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/session/manager/mod.rs"))
            .context("read Runtime message projection visibility")?;
    let runtime_background_task_registry =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/background_task_registry.rs"))
            .context("read Runtime background-task registry visibility")?;
    let runtime_background_task_spec =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/background_task_spec.rs"))
            .context("read Runtime background-task specification visibility")?;
    let runtime_background_task_state =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/background_task_state.rs"))
            .context("read Runtime background-task state visibility")?;
    let runtime_background_task_completion = fs::read_to_string(
        workspace.join("crates/agena-runtime/src/background_task_completion.rs"),
    )
    .context("read Runtime background-task completion visibility")?;
    let runtime_services =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/services.rs"))
            .context("read Runtime service-bundle visibility")?;
    let runtime_snapshot_metadata =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/snapshot.rs"))
            .context("read Runtime snapshot metadata visibility")?;
    let runtime_web = fs::read_to_string(workspace.join("crates/agena-runtime/src/web/mod.rs"))
        .context("read Runtime web-plugin visibility")?;
    let runtime_web_plugin =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/web/plugin.rs"))
            .context("read Runtime web-plugin implementation visibility")?;
    let runtime_plugin_service =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/plugin_runtime_service.rs"))
            .context("read Runtime plugin-service bridge visibility")?;
    let runtime_config_paths =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/config_paths.rs"))
            .context("read Runtime configuration-path visibility")?;
    let runtime_snapshot_registry =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/snapshot_registry.rs"))
            .context("read Runtime snapshot-registry visibility")?;
    let provider_catalog_model_id =
        fs::read_to_string(workspace.join("crates/agena-provider/src/catalog_model_id.rs"))
            .context("read provider catalog model-ID values")?;
    if [
        "agent", "agents", "config", "db", "error", "event", "message", "model_catalog",
        "permission", "plugins", "provider", "session", "tool", "web",
    ]
    .into_iter()
    .any(|module| runtime_root.contains(format!("pub mod {module};").as_str()))
        || runtime_session_module.contains("project_message_part")
        || runtime_session_manager.contains("use history::project_message_part;")
        || !runtime_root
            .contains("pub(crate) use background_task_registry::RuntimeBackgroundTaskRegistry;")
        || runtime_root.contains("pub use background_task_registry::RuntimeBackgroundTaskRegistry;")
        || !runtime_root.contains("pub(crate) use background_task_spec::RuntimeBackgroundTaskSpec;")
        || runtime_root.contains("pub use background_task_spec::RuntimeBackgroundTaskSpec;")
        || !runtime_root
            .contains("pub(crate) use background_task_state::RuntimeBackgroundTaskState;")
        || runtime_root.contains("pub use background_task_state::RuntimeBackgroundTaskState;")
        || !runtime_root.contains(
            "pub(crate) use background_task_completion::RuntimeBackgroundTaskCompletion;",
        )
        || runtime_root.contains("pub use background_task_completion::RuntimeBackgroundTaskCompletion;")
        || !runtime_background_task_registry.contains("pub(crate) struct RuntimeBackgroundTaskRegistry")
        || runtime_background_task_registry.contains("pub struct RuntimeBackgroundTaskRegistry")
        || !runtime_background_task_spec.contains("pub(crate) struct RuntimeBackgroundTaskSpec")
        || runtime_background_task_spec.contains("pub struct RuntimeBackgroundTaskSpec")
        || !runtime_background_task_state.contains("pub(crate) struct RuntimeBackgroundTaskState")
        || runtime_background_task_state.contains("pub struct RuntimeBackgroundTaskState")
        || !runtime_background_task_completion
            .contains("pub(crate) enum RuntimeBackgroundTaskCompletion")
        || runtime_background_task_completion.contains("pub enum RuntimeBackgroundTaskCompletion")
        || !runtime_root.contains("pub(crate) use services::RuntimeServiceBundle;")
        || runtime_root.contains("pub use services::RuntimeServiceBundle;")
        || !runtime_services.contains("pub(crate) struct RuntimeServiceBundle")
        || runtime_services.contains("pub struct RuntimeServiceBundle")
        || !runtime_root.contains("pub(crate) use snapshot::SnapshotMetadata;")
        || runtime_root.contains("pub use snapshot::SnapshotMetadata;")
        || !runtime_snapshot_metadata.contains("pub(crate) struct SnapshotMetadata")
        || runtime_snapshot_metadata.contains("pub struct SnapshotMetadata")
        || !runtime_root.contains("pub(crate) use web::{")
        || runtime_root.contains("pub use web::{")
        || !runtime_web.contains("pub(crate) use plugin::{WEB_PLUGIN_ID, WebPlugin};")
        || !runtime_web.contains("pub(crate) fn new_web_plugin()")
        || !runtime_web.contains("pub(crate) fn web_plugin_id()")
        || runtime_web.contains("new_web_plugin_with_user_agent")
        || !runtime_web_plugin.contains("pub(crate) const WEB_PLUGIN_ID")
        || !runtime_web_plugin.contains("pub(crate) struct WebConfig")
        || !runtime_web_plugin.contains("pub(crate) struct WebPlugin")
        || runtime_web_plugin.contains("fn with_user_agent(")
        || !runtime_root.contains("pub(crate) use plugin_runtime_service::dispatch_plugin_rpc;")
        || runtime_root.contains("RuntimePluginToolCatalogItem, dispatch_plugin_rpc,")
        || !runtime_plugin_service.contains("pub(crate) async fn dispatch_plugin_rpc(")
        || runtime_plugin_service.contains("pub async fn dispatch_plugin_rpc(")
        || !runtime_root.contains("pub(crate) use config_paths::{default_workspace_root, project_config_path};")
        || runtime_root.contains("pub use config_paths::{default_config_path, default_workspace_root, project_config_path};")
        || !runtime_config_paths.contains("pub(crate) fn default_workspace_root()")
        || !runtime_config_paths.contains("pub(crate) fn project_config_path(")
        || !runtime_root.contains("pub(crate) use snapshot_registry::snapshot_rift_binary;")
        || !runtime_snapshot_registry.contains("pub(crate) fn snapshot_rift_binary()")
        || runtime_root.contains("pub use model_catalog_curation::normalized_catalog_model_id;")
        || runtime_root.contains("catalog_model_id_for_raw")
        || !provider_catalog_model_id.contains("pub fn normalized_catalog_model_id")
        || !provider_catalog_model_id.contains("pub fn catalog_model_id_for_raw")
    {
        bail!(
            "Runtime implementation modules, message projection, background-task registry, service bundle, snapshot metadata, web plugin, plugin-host bridge, internal config paths, and snapshot binary lookup must remain crate-private"
        );
    }
    if app_settings_view.contains("fn settings_section_group_label") {
        bail!("final app must not restore settings-studio section-group presentation policy");
    }
    if !app_overlay_types.contains("state: SettingsStudioPresentation<SettingsPickerAction>")
        || !app_settings_input.contains("agena_tui::settings_studio::handle_key")
        || !app_settings_input.contains("SettingsStudioEffect::Activate")
        || !app_settings_selection.contains("dialog.state.select_query(query)")
        || !app_settings_builder.contains("SettingsStudioPresentation::new")
    {
        bail!("final app must adapt the TUI settings-studio reducer and presentation state");
    }

    let tui_agent_studio = workspace.join("crates/agena-tui/src/agent_studio.rs");
    let tui_agent_studio_source = fs::read_to_string(&tui_agent_studio)
        .with_context(|| format!("read {}", tui_agent_studio.display()))?;
    let tui_lib = fs::read_to_string(workspace.join("crates/agena-tui/src/lib.rs"))
        .context("read TUI module exports")?;
    if !tui_lib.contains("pub mod agent_studio;") {
        bail!("TUI agent-studio presentation module must remain exported");
    }
    for required in [
        "pub struct AgentStudioItem<A>",
        "pub struct AgentStudioPresentation<A>",
        "pub enum AgentStudioEffect",
        "pub fn handle_key<A>(",
    ] {
        if !tui_agent_studio_source.contains(required) {
            bail!("TUI agent-studio slice must retain `{required}`");
        }
    }
    let app_agent_input =
        fs::read_to_string(app_source.join("app/app_permissions/overlay_handlers.rs"))
            .context("read final-app agent-studio input adapter")?;
    if app_overlay_types.contains("struct AgentStudioItem")
        || !app_overlay_types.contains("presentation: AgentStudioPresentation<AgentStudioAction>")
        || !app_overlay_types.contains("editor: Option<AgentStudioEditor>")
        || !app_agent_input
            .contains("agena_tui::agent_studio::handle_key(&mut dialog.presentation, key)")
        || app_agent_input.contains("handle_structural_navigation_key(key, 10)")
    {
        bail!("final app must adapt the TUI agent-studio presentation reducer");
    }
    let application_runtime_dto =
        fs::read_to_string(workspace.join("crates/agena-application/src/dto/runtime.rs"))
            .context("read application agent presentation resources")?;
    let application_handle =
        fs::read_to_string(workspace.join("crates/agena-application/src/application.rs"))
            .context("read application agent projection boundary")?;
    let app_root =
        fs::read_to_string(app_source.join("app.rs")).context("read final-app type imports")?;
    let app_backend_workspace = fs::read_to_string(app_source.join("backend/backend_workspace.rs"))
        .context("read final-app agent backend adapter")?;
    if !application_runtime_dto.contains("pub struct RuntimeAgentProfileResource")
        || !application_runtime_dto.contains("impl From<agena_runtime::RuntimeAgentProfile>")
        || !application_runtime_dto.contains("impl From<agena_runtime::RuntimeAgentStatus>")
        || !application_runtime_dto.contains("pub struct TuiPreferencesResource")
        || !application_runtime_dto
            .contains("impl From<agena_runtime::RuntimeUiConfiguration> for TuiPreferencesResource")
        || !application_runtime_dto.contains("pub struct RuntimeSnapshotSummaryResource")
        || !application_handle.contains("pub fn agent_statuses(&self) -> Vec<RuntimeAgentResource>")
        || !application_handle.contains(
            "pub fn agent_profile(&self, name: &str) -> Option<RuntimeAgentProfileResource>",
        )
        || !application_handle.contains("pub fn default_agent_name(&self) -> Option<String>")
        || !application_handle.contains(
            "pub async fn runtime_snapshot_summary(&self) -> RuntimeSnapshotSummaryResource",
        )
        || !application_handle.contains(
            "pub fn tui_preferences(&self) -> Result<TuiPreferencesResource, ApplicationError>",
        )
        || app_root.contains("RuntimeAgentProfile as AgentProfile")
        || app_root.contains("RuntimeAgentStatus as AgentDescriptor")
        || app_root.contains("RuntimeAgentSelectionStatus as AgentSelectionConfig")
        || app_backend_workspace
            .contains("use agena_runtime::{RuntimeAgentProfile, RuntimeAgentStatus}")
        || !app_backend_workspace.contains("self.application.agent_statuses()")
        || !app_backend_workspace.contains("self.application.agent_profile(name)")
        || !app_backend_workspace.contains("self.application.default_agent_name()")
        || !app_backend_workspace.contains("self.application.runtime_snapshot_summary().await")
        || !app_backend_workspace.contains("status.provider_count")
        || !app_backend_workspace.contains("self.application\n            .tui_preferences()")
        || app_backend_workspace.contains("agena_runtime::RuntimeUiConfiguration")
        || app_backend_workspace.contains("status.provider_ids")
        || app_backend_workspace.contains(".runtime_status()")
    {
        bail!(
            "final app must consume Application agent/status/UI-preference projections instead of Runtime concrete values"
        );
    }
    let api_rest = fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/mod.rs"))
        .context("read API metrics transport adapter")?;
    if !application_runtime_dto.contains("pub struct RuntimeMetricsResource")
        || !application_runtime_dto.contains("impl From<agena_runtime::RuntimeMetricsSnapshot>")
        || !application_handle.contains("pub fn runtime_metrics(&self) -> RuntimeMetricsResource")
        || !application_handle.contains("self.runtime_control.runtime_metrics().into()")
        || application_handle.contains("agena_runtime::runtime_metrics_snapshot()")
        || !api_rest.contains("let runtime_metrics = state.runtime_metrics();")
        || api_rest.contains("agena_runtime::runtime_metrics_snapshot()")
    {
        bail!(
            "API metrics transport must consume the Application metrics projection rather than Runtime directly"
        );
    }
    let application_git_service =
        fs::read_to_string(workspace.join("crates/agena-application/src/service/git.rs"))
            .context("read application Git/snapshot projection")?;
    let runtime_control_port =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime_control_service.rs"))
            .context("read runtime snapshot-capability control port")?;
    let runtime_root = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
        .context("read Runtime root exports")?;
    let runtime_builder =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/builder.rs"))
            .context("read Runtime snapshot-capability control adapter")?;
    if !runtime_control_port.contains(
        "fn snapshot_backend_capabilities(&self, workspace: &Path) -> SnapshotBackendCapabilities;",
    ) || !application_handle.contains("RuntimeControlService::snapshot_backend_capabilities(")
        || !runtime_builder.contains("fn snapshot_backend_capabilities(")
        || application_git_service.contains("agena_runtime::snapshot_backend_capabilities")
        || !runtime_root
            .contains("pub(crate) use snapshot_capabilities::snapshot_backend_capabilities;")
        || runtime_root.contains("pub use snapshot_capabilities::snapshot_backend_capabilities;")
    {
        bail!(
            "Application snapshot status must use the composed Runtime control capability rather than a public process-probe helper"
        );
    }

    let tui_model_catalog = workspace.join("crates/agena-tui/src/model_catalog.rs");
    let tui_model_catalog_source = fs::read_to_string(&tui_model_catalog)
        .with_context(|| format!("read {}", tui_model_catalog.display()))?;
    if !tui_lib.contains("pub mod model_catalog;") {
        bail!("TUI model-catalog presentation module must remain exported");
    }
    for required in [
        "pub struct ModelCatalogItem",
        "pub struct ModelCatalogDetail",
        "pub struct ModelCatalogPresentation",
        "pub enum ModelCatalogEffect",
        "pub fn begin_query",
        "pub fn apply_page",
        "pub fn handle_key(",
    ] {
        if !tui_model_catalog_source.contains(required) {
            bail!("TUI model-catalog slice must retain `{required}`");
        }
    }
    let app_model_catalog_input = fs::read_to_string(app_source.join("app/app_overlays.rs"))
        .context("read final-app model-catalog input adapter")?;
    let app_model_catalog_events =
        fs::read_to_string(app_source.join("app/app_session_events/handlers.rs"))
            .context("read final-app model-catalog event adapter")?;
    let app_model_catalog_projection =
        fs::read_to_string(app_source.join("app/view/view_catalog_helpers.rs"))
            .context("read final-app model-catalog display projection adapter")?;
    let app_model_catalog_view = fs::read_to_string(app_source.join("app/view/view_studio.rs"))
        .context("read final-app model-catalog view adapter")?;
    if app_overlay_types.contains("workbench: ListWorkbenchState<CatalogModelResource")
        || app_overlay_types.contains("ModelCatalogPresentation<CatalogModelResource>")
        || !app_model_catalog_input
            .contains("agena_tui::model_catalog::handle_key(&mut dialog.presentation, key)")
        || !app_model_catalog_events.contains("dialog.presentation.apply_page(")
        || !app_model_catalog_events.contains("model_catalog_presentation_item(&self.i18n, entry)")
        || !app_model_catalog_projection.contains("-> ModelCatalogItem")
        || !app_model_catalog_projection.contains("-> ModelCatalogDetail")
        || app_model_catalog_projection.contains("fn model_catalog_detail_text")
        || !app_model_catalog_view.contains(".map(|entry| entry.detail.text())")
        || app_model_catalog_view.contains("model_catalog_detail_text")
    {
        bail!(
            "final app must adapt the opaque TUI model-catalog query, pagination, and display projection reducer"
        );
    }

    let tui_session_status = workspace.join("crates/agena-tui/src/session_status.rs");
    let tui_session_status_source = fs::read_to_string(&tui_session_status)
        .with_context(|| format!("read {}", tui_session_status.display()))?;
    if !tui_lib.contains("pub mod session_status;") {
        bail!("TUI session-status presentation module must remain exported");
    }
    for required in [
        "pub enum TokenUsageStatus",
        "pub fn token_usage_status(",
        "pub fn session_summary_status_parts(",
        "pub fn format_tokens_k(",
    ] {
        if !tui_session_status_source.contains(required) {
            bail!("TUI session-status display slice must retain `{required}`");
        }
    }
    let app_session_helpers = fs::read_to_string(app_source.join("app/app_session_helpers.rs"))
        .context("read final-app session-status helper owner")?;
    let app_status_context = fs::read_to_string(app_source.join("app/app_status_context.rs"))
        .context("read final-app session-status adapter")?;
    let runtime_context_budget =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/context_budget.rs"))
            .context("read Runtime context-budget policy")?;
    if app_session_helpers.contains("enum TokenUsageStatus")
        || app_session_helpers.contains("fn status_line_token_usage")
        || app_session_helpers.contains("fn session_summary_status_parts")
        || app_session_helpers.contains("fn format_token_progress_label")
        || app_session_helpers.contains("fn format_tokens_k")
        || !app_status_context.contains("agena_tui::session_status::token_usage_status(")
        || !app_status_context.contains("agena_tui::session_status::session_summary_status_parts(")
        || runtime_context_budget.contains("context_usage_percent_used")
        || runtime_root.contains("pub use context_budget::context_usage_percent_used;")
    {
        bail!(
            "terminal session-status display policy must remain TUI-owned without a Runtime percentage helper"
        );
    }

    let tui_permission_rule = workspace.join("crates/agena-tui/src/permission_rule_studio.rs");
    let tui_permission_rule_source = fs::read_to_string(&tui_permission_rule)
        .with_context(|| format!("read {}", tui_permission_rule.display()))?;
    if !tui_lib.contains("pub mod permission_rule_studio;") {
        bail!("TUI permission-rule-studio presentation module must remain exported");
    }
    for required in [
        "pub struct PermissionRuleStudioItem<A>",
        "pub struct PermissionRuleStudioPresentation<A>",
        "pub enum PermissionRuleStudioEffect",
        "pub fn handle_key<A>",
    ] {
        if !tui_permission_rule_source.contains(required) {
            bail!("TUI permission-rule-studio slice must retain `{required}`");
        }
    }
    let app_permission_rule_input =
        fs::read_to_string(app_source.join("app/app_permissions/overlay_handlers.rs"))
            .context("read final-app permission-rule-studio input adapter")?;
    if app_overlay_types.contains("struct PermissionRuleStudioItem")
        || app_overlay_types.contains("workbench: ListWorkbenchState<PermissionRuleStudioItem")
        || !app_overlay_types
            .contains("presentation: PermissionRuleStudioPresentation<PermissionRuleStudioAction>")
        || !app_permission_rule_input.contains("agena_tui::permission_rule_studio::handle_key(")
        || !app_permission_rule_input.contains("&mut dialog.presentation")
    {
        bail!("final app must adapt the TUI permission-rule-studio presentation reducer");
    }

    let tui_permission_prompt = workspace.join("crates/agena-tui/src/permission_prompt.rs");
    let tui_permission_prompt_source = fs::read_to_string(&tui_permission_prompt)
        .with_context(|| format!("read {}", tui_permission_prompt.display()))?;
    for required in [
        "pub enum PermissionPromptDecision",
        "pub enum PermissionPromptDetailsReturn",
        "pub enum PermissionPromptPage",
        "pub enum PermissionPromptLineTone",
        "pub struct PermissionPromptLine",
        "pub struct PermissionPromptContent",
        "pub struct PermissionPromptPresentation",
        "pub enum PermissionPromptEffect",
        "pub fn choice_count",
        "pub fn choice_decision_label",
        "pub fn title",
        "pub fn footer",
        "pub fn open_scope",
        "pub fn open_details",
        "pub fn active_content",
        "pub fn handle_key",
        "pub fn render_overlay",
    ] {
        if !tui_permission_prompt_source.contains(required) {
            bail!("TUI permission-prompt slice must retain `{required}`");
        }
    }
    for forbidden in [
        "agena_api",
        "agena_application",
        "agena_domain",
        "agena_runtime",
        "agena_storage",
        "agena_provider",
        "SeaOrm",
        "DatabaseConnection",
        "PermissionRequest",
        "PermissionAction",
    ] {
        if tui_permission_prompt_source.contains(forbidden) {
            bail!(
                "TUI permission-prompt presentation must not retain a concrete dependency `{forbidden}`"
            );
        }
    }
    let app_permission_prompt_input =
        fs::read_to_string(app_source.join("app/app_permissions/overlay_handlers.rs"))
            .context("read final-app permission-prompt input adapter")?;
    let app_permission_prompt_view =
        fs::read_to_string(app_source.join("app/view/view_overlays/overlay_core.rs"))
            .context("read final-app permission-prompt view adapter")?;
    let app_permission_prompt_projection =
        fs::read_to_string(app_source.join("app/app_permission_display.rs"))
            .context("read final-app permission-prompt display projection")?;
    let app_permission_prompt_legacy_view =
        fs::read_to_string(app_source.join("app/view/view_permission_helpers.rs"))
            .context("read final-app legacy permission-prompt renderer helpers")?;
    for forbidden in [
        "enum PermissionOverlayPage",
        "enum PermissionOverlayDetailsReturn",
        "enum PermissionOverlayDecision",
        "page: PermissionOverlayPage",
    ] {
        if app_overlay_types.contains(forbidden) {
            bail!("final app must not restore local permission-prompt state `{forbidden}`");
        }
    }
    if !app_overlay_types.contains("presentation: PermissionPromptPresentation")
        || !app_permission_prompt_input.contains("agena_tui::permission_prompt::handle_key")
        || !app_permission_prompt_input.contains("PermissionPromptEffect::Activate")
        || !app_permission_prompt_projection.contains("fn permission_prompt_content(")
        || !app_permission_prompt_view.contains("agena_tui::permission_prompt::render_overlay(")
        || app_permission_prompt_view.contains("fn render_permission_overlay")
        || app_permission_prompt_legacy_view.contains("fn permission_overlay_body_lines")
        || app_permission_prompt_legacy_view.contains("fn permission_overlay_choice_lines")
    {
        bail!(
            "final app must adapt the complete TUI permission-prompt content, view, navigation, and selection state"
        );
    }

    let tui_permission_studio = workspace.join("crates/agena-tui/src/permission_studio.rs");
    let tui_permission_studio_source = fs::read_to_string(&tui_permission_studio)
        .with_context(|| format!("read {}", tui_permission_studio.display()))?;
    for required in [
        "pub struct PermissionStudioNavItem",
        "pub enum PermissionStudioPaneFocus",
        "pub enum PermissionStudioSectionId",
        "pub enum PermissionStudioPage",
        "pub type PermissionStudioFocus",
        "pub fn nav_items",
        "pub fn nav_index_for_page",
        "pub fn nav_normalize_selection",
        "pub fn nav_move_step",
        "pub fn next",
    ] {
        if !tui_permission_studio_source.contains(required) {
            bail!("TUI permission-studio navigation slice must retain `{required}`");
        }
    }
    let app_permission_types = fs::read_to_string(app_source.join("app/app_types/overlays.rs"))
        .context("read final-app permission-studio presentation types")?;
    let app_permission_navigation =
        fs::read_to_string(app_source.join("app/app_permission_helpers/navigation.rs"))
            .context("read final-app permission-studio navigation adapter")?;
    let app_permission_input =
        fs::read_to_string(app_source.join("app/app_permissions/overlay_handlers.rs"))
            .context("read final-app permission-studio input adapter")?;
    for forbidden in [
        "struct PermissionStudioNavItem",
        "enum PermissionStudioPaneFocus",
        "enum PermissionStudioSectionId",
        "enum PermissionStudioPage",
        "type PermissionStudioFocus =",
        "fn permission_studio_nav_items",
        "fn permission_studio_nav_index_for_page",
        "fn permission_studio_nav_normalize_selection",
        "fn permission_studio_nav_move_step",
        "fn move_permission_studio_pane_focus",
    ] {
        if app_permission_types.contains(forbidden)
            || app_permission_navigation.contains(forbidden)
            || app_permission_input.contains(forbidden)
        {
            bail!("final app must not restore permission-studio navigation owner `{forbidden}`");
        }
    }
    if !app_permission_types.contains("agena_tui::permission_studio::{")
        || !app_permission_navigation.contains("agena_tui::permission_studio::{")
        || !app_permission_input.contains("dialog.pane_focus.next()")
    {
        bail!("final app must adapt the TUI permission-studio navigation reducer");
    }

    let tui_user_input = workspace.join("crates/agena-tui/src/user_input.rs");
    let tui_user_input_source = fs::read_to_string(&tui_user_input)
        .with_context(|| format!("read {}", tui_user_input.display()))?;
    for required in [
        "pub struct UserInputOptionPresentation",
        "pub struct UserInputQuestionPresentation",
        "pub struct UserInputOverlayPresentation",
        "pub struct UserInputAnswerDraft",
        "pub struct UserInputReviewPresentation",
        "pub struct UserInputPresentation",
        "pub enum UserInputEffect",
        "pub fn handle_key",
        "pub fn insert_custom_text",
        "pub fn focus_question",
        "pub fn render_overlay",
    ] {
        if !tui_user_input_source.contains(required) {
            bail!("TUI user-input slice must retain `{required}`");
        }
    }
    let app_user_input = fs::read_to_string(app_source.join("app/app_user_input.rs"))
        .context("read final-app user-input adapter")?;
    let app_user_input_overlay =
        fs::read_to_string(app_source.join("app/view/view_overlays/overlay_core.rs"))
            .context("read final-app user-input renderer adapter")?;
    if app_composer_types.contains("struct UserInputAnswerDraft")
        || app_overlay_types.contains("answers: BTreeMap<String, UserInputAnswerDraft>")
        || app_overlay_types.contains("state: QuestionFlowState")
        || app_overlay_types.contains("editing_custom: bool")
        || app_overlay_types.contains("custom_input: Editor")
        || app_overlay_types.contains("review: UserInputReviewPresentation")
        || !app_overlay_types.contains("presentation: UserInputPresentation")
        || !app_user_input.contains("dialog.presentation.handle_key(key)")
        || !app_user_input.contains("UserInputEffect::Submit")
        || !app_user_input.contains("UserInputEffect::Cancel")
        || !app_user_input_overlay.contains("agena_tui::user_input::render_overlay(")
        || app_user_input_overlay.contains("fn render_user_input_overlay")
        || app_user_input_overlay.contains("fn render_user_input_review_overlay")
        || app_source
            .join("app/view/view_user_input_helpers.rs")
            .exists()
    {
        bail!("final app must adapt the complete TUI user-input presentation reducer");
    }
    for forbidden in [
        "fn handle_user_input_question_key",
        "fn handle_user_input_review_key",
        "fn move_user_input_question",
        "fn move_user_input_option",
        "fn move_user_input_tab",
        "fn toggle_user_input_option",
        "fn select_user_input_option",
        "fn begin_user_input_custom_edit",
        "fn commit_user_input_custom_values",
        "fn clear_user_input_answer",
        "fn user_input_option_row_count",
        "fn preferred_user_input_option_row",
        "fn selected_user_input_row_is_custom",
    ] {
        if app_user_input.contains(forbidden) {
            bail!("final app must not restore user-input presentation reducer `{forbidden}`");
        }
    }

    let tui_status_line = workspace.join("crates/agena-tui/src/status_line.rs");
    let tui_status_line_source = fs::read_to_string(&tui_status_line)
        .with_context(|| format!("read {}", tui_status_line.display()))?;
    for required in [
        "pub enum StatusLineEffect",
        "pub struct StatusLinePresentation",
        "pub fn from_config",
        "pub fn tick",
        "pub fn apply_refresh",
        "pub fn text",
    ] {
        if !tui_status_line_source.contains(required) {
            bail!("TUI status-line slice must retain `{required}`");
        }
    }
    if app_types.contains("struct StatusLineState")
        || !app_types.contains("agena_tui::status_line::StatusLinePresentation")
    {
        bail!("final app must consume the TUI status-line presentation state");
    }
    let app_status_execution =
        fs::read_to_string(app_source.join("app/app_session_interactive/execution.rs"))
            .context("read final-app status-line effect adapter")?;
    let app_status_dispatch =
        fs::read_to_string(app_source.join("app/app_session_events/dispatch.rs"))
            .context("read final-app status-line effect result adapter")?;
    if !app_status_execution.contains("status_line.tick(now)")
        || !app_status_execution.contains("StatusLineEffect::Refresh")
        || !app_status_dispatch.contains("status_line.apply_refresh(output)")
    {
        bail!("final app must adapt TUI status-line effects instead of owning its scheduler");
    }

    let tui_usage = workspace.join("crates/agena-tui/src/usage.rs");
    let tui_usage_source =
        fs::read_to_string(&tui_usage).with_context(|| format!("read {}", tui_usage.display()))?;
    for required in [
        "pub enum UsageDashboardView",
        "pub enum UsageDashboardSort",
        "pub enum UsageDashboardControl",
        "pub struct UsageDashboardPresentation",
        "pub enum UsageDashboardEffect",
        "pub struct UsageDashboardData",
        "pub struct UsageDashboardRow",
        "pub struct UsageDashboardSessionLink",
        "pub fn usage_dashboard_sort_order",
        "pub fn render_usage_dashboard",
        "pub fn activate",
        "pub fn move_selection",
        "pub fn usage_dashboard_view_label",
        "pub fn usage_dashboard_sort_label",
    ] {
        if !tui_usage_source.contains(required) {
            bail!("TUI usage presentation slice must retain `{required}`");
        }
    }
    for forbidden in [
        "enum UsageDashboardView",
        "enum UsageDashboardSort",
        "enum UsageDashboardControl",
        "pub(super) view: UsageDashboardView",
        "pub(super) sort: UsageDashboardSort",
        "pub(super) provider_filter: Option<String>",
        "pub(super) model_filter: Option<String>",
        "pub(super) selected: usize",
    ] {
        if app_types.contains(forbidden) {
            bail!("final app must consume the TUI usage presentation value `{forbidden}`");
        }
    }
    let app_usage = fs::read_to_string(app_source.join("app/app_usage.rs"))
        .context("read final-app usage effect adapter")?;
    let app_usage_view = fs::read_to_string(app_source.join("app/view/view_usage.rs"))
        .context("read final-app usage renderer")?;
    if !app_usage.contains("UsageDashboardEffect")
        || !app_usage.contains("state.presentation.activate(")
        || !app_usage.contains("state.presentation.move_selection")
        || !app_usage.contains("state.data = Some(usage_dashboard_data(&stats))")
        || !app_usage.contains("agena_tui::usage::")
        || !app_usage_view.contains("agena_tui::usage::render_usage_dashboard")
        || app_usage_view.contains("UsageStats")
        || app_usage_view.contains("UsageTotals")
        || app_usage_view.contains("render_usage_table")
    {
        bail!(
            "final app usage feature must project Runtime data into, then render through, its TUI owner"
        );
    }

    let app_plugin_backend = fs::read_to_string(app_source.join("backend/backend_plugins.rs"))
        .context("read final-app plugin and Git adapter")?;
    for required in [
        "self.application.snapshot_status()",
        ".git_commit(agena_application::dto::GitCommitRequest { message })",
        ".git_create_pull_request(agena_application::dto::GitPullRequestCreateRequest",
    ] {
        if !app_plugin_backend.contains(required) {
            bail!("final app must consume Application Git/snapshot use case `{required}`");
        }
    }
    for forbidden in [
        "SessionExecutionControl::snapshot_registry",
        "list_active_snapshots",
        "list_managed_snapshots",
        "snapshot_backend_capabilities",
        "Command::new(\"git\")",
        "Command::new(\"gh\")",
    ] {
        if app_plugin_backend.contains(forbidden) {
            bail!("final app Git/snapshot adapter must not retain duplicated policy `{forbidden}`");
        }
    }
    let app_session_backend = fs::read_to_string(app_source.join("backend/backend_session.rs"))
        .context("read final-app session command adapter")?;
    if !app_session_backend.contains("ApiCommand::CancelRun")
        || app_session_backend.contains("session_execution_control()")
        || app_session_backend.contains("cancel_active_execution(session_id)")
    {
        bail!(
            "final app cancellation must use the shared Application command rather than Runtime execution control"
        );
    }
    Ok(())
}

fn assert_provider_protocol_id_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let protocol_ids = workspace.join("crates/agena-provider/src/protocol_ids.rs");
    let provider_source = fs::read_to_string(&protocol_ids)
        .with_context(|| format!("read {}", protocol_ids.display()))?;
    for required in [
        "pub struct ProviderStreamKey",
        "pub struct ModelToolCallId",
        "pub struct ProviderItemId",
        "pub fn openai_responses_call_id",
    ] {
        if !provider_source.contains(required) {
            bail!("provider protocol identifiers must retain `{required}`");
        }
    }
    if workspace
        .join("crates/agena-runtime/src/provider/protocol_ids.rs")
        .exists()
    {
        bail!("Core must not retain provider protocol identifier implementation");
    }
    Ok(())
}

fn assert_provider_tool_stream_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tool_stream = workspace.join("crates/agena-provider/src/tool_stream.rs");
    let provider_source = fs::read_to_string(&tool_stream)
        .with_context(|| format!("read {}", tool_stream.display()))?;
    for required in [
        "pub struct ToolStreamInput",
        "pub enum ToolStreamUpdate",
        "pub struct ToolStreamAccumulator",
        "pub enum ToolStreamError",
    ] {
        if !provider_source.contains(required) {
            bail!("provider tool-stream contract must retain `{required}`");
        }
    }
    if workspace
        .join("crates/agena-runtime/src/provider/tool_stream.rs")
        .exists()
    {
        bail!("Core must not retain provider tool-stream accumulation implementation");
    }
    Ok(())
}

fn assert_provider_prompt_tool_envelope_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let envelope = workspace.join("crates/agena-provider/src/prompt_tool_envelope.rs");
    let provider_source =
        fs::read_to_string(&envelope).with_context(|| format!("read {}", envelope.display()))?;
    for required in [
        "pub struct PromptToolCallsEnvelope",
        "pub struct PromptToolCall",
        "pub struct PromptToolDefinition",
        "pub struct PromptToolResult",
    ] {
        if !provider_source.contains(required) {
            bail!("provider prompt-tool envelope must retain `{required}`");
        }
    }
    let core_source = fs::read_to_string(
        workspace.join("crates/agena-runtime/src/provider/prompt_tool_transport.rs"),
    )
    .context("read Core prompt-tool transport")?;
    for forbidden in [
        "struct PromptToolCallsEnvelope",
        "struct PromptToolCall",
        "struct PromptToolDefinition",
        "struct PromptToolResult",
    ] {
        if core_source.contains(forbidden) {
            bail!("Core must not define prompt-tool envelope value `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_prompt_tool_decoder_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let decoder = workspace.join("crates/agena-provider/src/prompt_tool_decoder.rs");
    let provider_source =
        fs::read_to_string(&decoder).with_context(|| format!("read {}", decoder.display()))?;
    for required in [
        "pub enum PromptToolDecodedItem",
        "pub struct PromptToolTextDecoder",
        "pub fn decode_prompt_tool_calls",
    ] {
        if !provider_source.contains(required) {
            bail!("provider prompt-tool decoder must retain `{required}`");
        }
    }
    let core_source = fs::read_to_string(
        workspace.join("crates/agena-runtime/src/provider/prompt_tool_transport.rs"),
    )
    .context("read Core prompt-tool transport")?;
    for forbidden in [
        "enum DecodedItem",
        "struct PromptToolTextDecoder",
        "enum DecoderState",
        "fn decode_calls",
    ] {
        if core_source.contains(forbidden) {
            bail!("Core must not define prompt-tool decoder `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_anthropic_text_wire_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wire = workspace.join("crates/agena-provider/src/anthropic_wire_text.rs");
    let provider_source =
        fs::read_to_string(&wire).with_context(|| format!("read {}", wire.display()))?;
    for required in [
        "pub struct AnthropicTextBlock",
        "pub struct AnthropicBinarySource",
        "pub fn tool_use",
        "pub fn tool_result",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Anthropic text wire must retain `{required}`");
        }
    }
    let core_wire =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/anthropic.rs"))
            .context("read Core Anthropic wire adapter")?;
    for forbidden in [
        "pub(crate) struct AnthropicTextBlock",
        "pub(crate) struct AnthropicBinarySource",
    ] {
        if core_wire.contains(forbidden) {
            bail!("Core Anthropic wire adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_anthropic_wire_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wire = workspace.join("crates/agena-provider/src/anthropic_wire.rs");
    let provider_source =
        fs::read_to_string(&wire).with_context(|| format!("read {}", wire.display()))?;
    for required in [
        "pub struct AnthropicMessagesRequest",
        "pub enum AnthropicModelListResponse",
        "pub struct AnthropicMessagesResponse",
        "pub struct AnthropicUsage",
        "pub enum AnthropicSseEvent",
        "pub struct AnthropicToolCallState",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Anthropic wire must retain `{required}`");
        }
    }
    if workspace
        .join("crates/agena-runtime/src/provider/anthropic/anthropic_wire.rs")
        .exists()
    {
        bail!("Core must not retain the Anthropic protocol wire module");
    }
    Ok(())
}

fn assert_provider_anthropic_thinking_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let thinking = workspace.join("crates/agena-provider/src/anthropic_thinking.rs");
    let provider_source =
        fs::read_to_string(&thinking).with_context(|| format!("read {}", thinking.display()))?;
    for required in [
        "pub struct AnthropicThinkingBlockState",
        "pub struct AnthropicThinkingParts",
        "pub fn anthropic_thinking_parts",
        "pub fn map_anthropic_usage",
        "pub fn merge_anthropic_usage",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Anthropic thinking policy must retain `{required}`");
        }
    }
    if workspace
        .join("crates/agena-runtime/src/provider/anthropic/anthropic_thinking.rs")
        .exists()
    {
        bail!("Core must not retain Anthropic thinking protocol policy");
    }
    Ok(())
}

fn assert_provider_gemini_thinking_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let thinking = workspace.join("crates/agena-provider/src/gemini_thinking.rs");
    let provider_source =
        fs::read_to_string(&thinking).with_context(|| format!("read {}", thinking.display()))?;
    for required in [
        "pub struct GeminiThinkingConfig",
        "pub fn gemini_thinking_config",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Gemini thinking policy must retain `{required}`");
        }
    }
    let core_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/gemini.rs"))
            .context("read Core Gemini adapter")?;
    for forbidden in ["fn gemini_thinking_config", "struct GeminiThinkingConfig"] {
        if core_source.contains(forbidden) {
            bail!("Core Gemini adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_gemini_usage_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let usage = workspace.join("crates/agena-provider/src/gemini_usage.rs");
    let provider_source =
        fs::read_to_string(&usage).with_context(|| format!("read {}", usage.display()))?;
    for required in [
        "pub struct GeminiUsageMetadata",
        "pub fn gemini_usage_to_completion",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Gemini usage must retain `{required}`");
        }
    }
    let core_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/gemini.rs"))
            .context("read Core Gemini adapter")?;
    for forbidden in ["struct GeminiUsageMetadata", "fn map_gemini_usage"] {
        if core_source.contains(forbidden) {
            bail!("Core Gemini adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_gemini_model_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let models = workspace.join("crates/agena-provider/src/gemini_models.rs");
    let provider_source =
        fs::read_to_string(&models).with_context(|| format!("read {}", models.display()))?;
    for required in [
        "pub struct GeminiModelListResponse",
        "pub struct GeminiModel",
        "pub fn metadata",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Gemini model projection must retain `{required}`");
        }
    }
    let core_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/gemini.rs"))
            .context("read Core Gemini adapter")?;
    for forbidden in ["struct GeminiModelListResponse", "struct GeminiModel"] {
        if core_source.contains(forbidden) {
            bail!("Core Gemini adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_gemini_content_wire_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wire = workspace.join("crates/agena-provider/src/gemini_content_wire.rs");
    let provider_source =
        fs::read_to_string(&wire).with_context(|| format!("read {}", wire.display()))?;
    for required in [
        "pub struct GeminiContent",
        "pub struct GeminiPart",
        "pub struct GeminiFunctionCall",
        "pub struct GeminiFunctionResponse",
        "pub struct GeminiInlineData",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Gemini content wire must retain `{required}`");
        }
    }
    let core_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/gemini.rs"))
            .context("read Core Gemini adapter")?;
    for forbidden in [
        "struct GeminiContent",
        "struct GeminiPart",
        "struct GeminiFunctionCall",
        "struct GeminiFunctionResponse",
        "struct GeminiInlineData",
    ] {
        if core_source.contains(forbidden) {
            bail!("Core Gemini adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_gemini_request_wire_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wire = workspace.join("crates/agena-provider/src/gemini_request_wire.rs");
    let provider_source =
        fs::read_to_string(&wire).with_context(|| format!("read {}", wire.display()))?;
    for required in [
        "pub struct GeminiGenerateRequest",
        "pub struct GeminiGenerationConfig",
        "pub struct GeminiFunctionDeclaration",
        "pub struct GeminiLiveConversationRequest",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Gemini request wire must retain `{required}`");
        }
    }
    let core_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/gemini.rs"))
            .context("read Core Gemini adapter")?;
    for forbidden in [
        "struct GeminiGenerateRequest",
        "struct GeminiGenerationConfig",
        "struct GeminiLiveConversationRequest",
    ] {
        if core_source.contains(forbidden) {
            bail!("Core Gemini adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_gemini_response_wire_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wire = workspace.join("crates/agena-provider/src/gemini_response_wire.rs");
    let provider_source =
        fs::read_to_string(&wire).with_context(|| format!("read {}", wire.display()))?;
    for required in [
        "pub struct GeminiGenerateResponse",
        "pub struct GeminiCandidate",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Gemini response wire must retain `{required}`");
        }
    }
    let core_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/gemini.rs"))
            .context("read Core Gemini adapter")?;
    for forbidden in ["struct GeminiGenerateResponse", "struct GeminiCandidate"] {
        if core_source.contains(forbidden) {
            bail!("Core Gemini adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_gemini_live_response_wire_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wire = workspace.join("crates/agena-provider/src/gemini_live_response_wire.rs");
    let provider_source =
        fs::read_to_string(&wire).with_context(|| format!("read {}", wire.display()))?;
    for required in [
        "pub struct GeminiLiveServerMessage",
        "pub struct GeminiLiveServerContent",
        "pub struct GeminiLiveToolCall",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Gemini Live response wire must retain `{required}`");
        }
    }
    let core_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/gemini.rs"))
            .context("read Core Gemini adapter")?;
    for forbidden in [
        "struct GeminiLiveServerMessage",
        "struct GeminiLiveServerContent",
        "struct GeminiLiveToolCall",
    ] {
        if core_source.contains(forbidden) {
            bail!("Core Gemini adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_ollama_wire_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wire = workspace.join("crates/agena-provider/src/ollama_wire.rs");
    let provider_source =
        fs::read_to_string(&wire).with_context(|| format!("read {}", wire.display()))?;
    for required in [
        "pub struct OllamaTagsResponse",
        "pub struct OllamaChatRequest",
        "pub struct OllamaChatResponse",
        "pub struct OllamaToolDefinition",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Ollama wire must retain `{required}`");
        }
    }
    let core_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/ollama.rs"))
            .context("read Core Ollama adapter")?;
    for forbidden in [
        "struct OllamaTagsResponse",
        "struct OllamaChatRequest",
        "struct OllamaChatResponse",
    ] {
        if core_source.contains(forbidden) {
            bail!("Core Ollama adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_ollama_usage_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let usage = workspace.join("crates/agena-provider/src/ollama_usage.rs");
    let provider_source =
        fs::read_to_string(&usage).with_context(|| format!("read {}", usage.display()))?;
    if !provider_source.contains("pub fn ollama_usage_to_completion") {
        bail!("provider Ollama usage must retain its normalized conversion");
    }
    let core_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/ollama.rs"))
            .context("read Core Ollama adapter")?;
    if core_source.contains("fn usage_from_response") {
        bail!("Core Ollama adapter must not retain usage normalization");
    }
    Ok(())
}

fn assert_provider_copilot_model_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let copilot_models = workspace.join("crates/agena-provider/src/copilot_models.rs");
    let provider_source = fs::read_to_string(&copilot_models)
        .with_context(|| format!("read {}", copilot_models.display()))?;
    for required in [
        "pub struct CopilotModelExtension",
        "pub fn visible",
        "pub fn metadata",
        "pub fn capabilities",
    ] {
        if !provider_source.contains(required) {
            bail!("provider Copilot model projection must retain `{required}`");
        }
    }
    if workspace
        .join("crates/agena-runtime/src/provider/copilot_models.rs")
        .exists()
    {
        bail!("Core must not retain Copilot model projection implementation");
    }
    Ok(())
}

fn assert_provider_tool_mode_policy_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tool_mode = workspace.join("crates/agena-provider/src/tool_mode_policy.rs");
    let provider_source =
        fs::read_to_string(&tool_mode).with_context(|| format!("read {}", tool_mode.display()))?;
    for required in [
        "pub fn apply_configured_tool_request",
        "pub fn prepare_disabled_tool_request",
        "pub fn strip_provider_native_tool_body_fields",
        "pub fn validate_disabled_tool_response",
    ] {
        if !provider_source.contains(required) {
            bail!("provider tool-mode policy must retain `{required}`");
        }
    }
    let core_tool_mode =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/tool_mode.rs"))
            .context("read Core provider stream guard")?;
    for forbidden in [
        "CompletionRequest",
        "apply_configured_request",
        "prepare_disabled_request",
    ] {
        if core_tool_mode.contains(forbidden) {
            bail!("Core tool-mode stream guard must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_openai_responses_wire_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wire = workspace.join("crates/agena-provider/src/openai_responses_wire.rs");
    let provider_source =
        fs::read_to_string(&wire).with_context(|| format!("read {}", wire.display()))?;
    for required in [
        "pub struct OpenAiResponsesResponse",
        "pub struct OpenAiOutputItem",
        "pub struct OpenAiUsage",
        "pub fn openai_responses_reasoning_delta",
    ] {
        if !provider_source.contains(required) {
            bail!("provider OpenAI Responses wire contract must retain `{required}`");
        }
    }
    let core_wire = fs::read_to_string(
        workspace.join("crates/agena-runtime/src/provider/openai/openai_response_types.rs"),
    )
    .context("read Core OpenAI response adapter helpers")?;
    for forbidden in [
        "pub(super) struct OpenAiResponsesResponse",
        "pub(super) struct OpenAiOutputItem",
        "pub(super) struct OpenAiUsage",
    ] {
        if core_wire.contains(forbidden) {
            bail!("Core OpenAI response adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_openai_chat_usage_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let usage = workspace.join("crates/agena-provider/src/openai_chat_usage.rs");
    let provider_source =
        fs::read_to_string(&usage).with_context(|| format!("read {}", usage.display()))?;
    for required in [
        "pub struct ChatUsage",
        "pub struct ChatOutputTokensDetails",
        "pub struct ChatInputTokensDetails",
        "pub fn chat_usage_to_completion",
    ] {
        if !provider_source.contains(required) {
            bail!("provider OpenAI Chat usage contract must retain `{required}`");
        }
    }
    let core_wire =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/chat_wire.rs"))
            .context("read Core Chat wire adapter helpers")?;
    for forbidden in [
        "pub(crate) struct ChatUsage",
        "pub(crate) struct ChatOutputTokensDetails",
        "pub(crate) struct ChatInputTokensDetails",
    ] {
        if core_wire.contains(forbidden) {
            bail!("Core Chat wire adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_provider_openai_chat_response_format_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let format = workspace.join("crates/agena-provider/src/openai_chat_response_format.rs");
    let provider_source =
        fs::read_to_string(&format).with_context(|| format!("read {}", format.display()))?;
    for required in [
        "pub enum ChatResponseFormat",
        "pub struct ChatJsonSchemaSpec",
        "pub fn openai_chat_response_format",
    ] {
        if !provider_source.contains(required) {
            bail!("provider OpenAI Chat response format must retain `{required}`");
        }
    }
    let core_wire =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/chat_wire.rs"))
            .context("read Core Chat wire adapter helpers")?;
    if core_wire.contains("pub(crate) enum ChatResponseFormat") {
        bail!("Core Chat wire adapter must not retain ChatResponseFormat");
    }
    Ok(())
}

fn assert_provider_openai_chat_reasoning_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let reasoning = workspace.join("crates/agena-provider/src/openai_chat_reasoning.rs");
    let provider_source =
        fs::read_to_string(&reasoning).with_context(|| format!("read {}", reasoning.display()))?;
    for required in [
        "pub fn openai_chat_reasoning_effort",
        "pub fn openai_chat_supports_reasoning_effort",
    ] {
        if !provider_source.contains(required) {
            bail!("provider OpenAI Chat reasoning policy must retain `{required}`");
        }
    }
    let core_wire =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/chat_wire.rs"))
            .context("read Core Chat wire adapter helpers")?;
    if core_wire.contains("pub(crate) fn reasoning_effort") {
        bail!("Core Chat wire adapter must not retain reasoning-effort policy");
    }
    Ok(())
}

fn assert_provider_openai_chat_response_wire_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wire = workspace.join("crates/agena-provider/src/openai_chat_response_wire.rs");
    let provider_source =
        fs::read_to_string(&wire).with_context(|| format!("read {}", wire.display()))?;
    for required in [
        "pub struct ChatCompletionResponse",
        "pub struct ChatCompletionChoice",
        "pub struct ChatDeltaOrMessage",
        "pub struct ChatToolCallWire",
    ] {
        if !provider_source.contains(required) {
            bail!("provider OpenAI Chat response wire must retain `{required}`");
        }
    }
    let core_wire =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/chat_wire.rs"))
            .context("read Core Chat wire adapter helpers")?;
    if core_wire.contains("pub(crate) struct ChatCompletionResponse") {
        bail!("Core Chat wire adapter must not retain ChatCompletionResponse");
    }
    Ok(())
}

fn assert_provider_openai_chat_tool_definition_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let definition = workspace.join("crates/agena-provider/src/openai_chat_tool_definition.rs");
    let provider_source = fs::read_to_string(&definition)
        .with_context(|| format!("read {}", definition.display()))?;
    for required in [
        "pub struct ChatToolDefinition",
        "pub struct ChatFunctionDefinition",
    ] {
        if !provider_source.contains(required) {
            bail!("provider OpenAI Chat tool definition must retain `{required}`");
        }
    }
    let core_wire =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/chat_wire.rs"))
            .context("read Core Chat wire adapter helpers")?;
    if core_wire.contains("pub(crate) struct ChatToolDefinition") {
        bail!("Core Chat wire adapter must not retain ChatToolDefinition");
    }
    Ok(())
}

fn assert_provider_openai_chat_request_wire_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (relative, required) in [
        (
            "openai_chat_completion_request.rs",
            "pub struct ChatCompletionRequest",
        ),
        ("openai_chat_message.rs", "pub struct ChatMessage"),
        (
            "openai_chat_stream_options.rs",
            "pub struct ChatStreamOptions",
        ),
        (
            "openai_chat_tool_call_request.rs",
            "pub struct ChatToolCallRequest",
        ),
    ] {
        let path = workspace.join("crates/agena-provider/src").join(relative);
        let source =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        if !source.contains(required) {
            bail!("provider OpenAI Chat request wire must retain `{required}` in {relative}");
        }
    }
    let core_wire =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/chat_wire.rs"))
            .context("read Core Chat wire adapter helpers")?;
    for forbidden in [
        "pub(crate) struct ChatCompletionRequest",
        "pub(crate) struct ChatMessage",
        "pub(crate) struct ChatStreamOptions",
        "pub(crate) struct ChatToolCallRequest",
    ] {
        if core_wire.contains(forbidden) {
            bail!("Core Chat wire adapter must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_runtime_provider_sse_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime_sse = workspace.join("crates/agena-runtime/src/provider_sse.rs");
    let runtime_source = fs::read_to_string(&runtime_sse)
        .with_context(|| format!("read {}", runtime_sse.display()))?;
    for required in [
        "pub enum JsonEventPayload",
        "pub enum ProviderJsonStreamError",
        "pub fn json_events_with_done",
        "pub fn json_lines",
    ] {
        if !runtime_source.contains(required) {
            bail!("Runtime provider SSE support must retain `{required}`");
        }
    }
    if workspace
        .join("crates/agena-runtime/src/provider/sse.rs")
        .exists()
    {
        bail!("Core must not retain provider SSE decoding implementation");
    }
    Ok(())
}

fn assert_runtime_config_value_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let values = workspace.join("crates/agena-runtime/src/config_values.rs");
    let runtime_source =
        fs::read_to_string(&values).with_context(|| format!("read {}", values.display()))?;
    for required in [
        "pub use self::provider::*",
        "pub use self::provider_native_tools::*",
        "pub use self::resolved::*",
        "pub use self::runtime::*",
    ] {
        if !runtime_source.contains(required) {
            bail!("Runtime configuration values must retain `{required}`");
        }
    }
    for relative in [
        "crates/agena-runtime/src/config_values/provider.rs",
        "crates/agena-runtime/src/config_values/resolved.rs",
        "crates/agena-runtime/src/config_values/runtime.rs",
    ] {
        let source = fs::read_to_string(workspace.join(relative))
            .with_context(|| format!("read Runtime configuration values {relative}"))?;
        if !source.contains("pub struct") && !source.contains("pub enum") {
            bail!("Runtime configuration values must define data in {relative}");
        }
    }
    if workspace
        .join("crates/agena-runtime/src/config/types.rs")
        .exists()
        || workspace
            .join("crates/agena-runtime/src/config/types")
            .exists()
    {
        bail!("Core must not retain configuration value definitions");
    }
    if workspace
        .join("crates/agena-runtime/src/config/overlay.rs")
        .exists()
    {
        bail!("Core must not retain provider configuration-overlay implementation");
    }
    let core_config = fs::read_to_string(workspace.join("crates/agena-runtime/src/config/mod.rs"))
        .context("read Core configuration module")?;
    if core_config.contains("pub use agena_runtime::{") {
        bail!("Core configuration must not publicly re-export Runtime configuration values");
    }
    let core_edit = fs::read_to_string(workspace.join("crates/agena-runtime/src/config/edit.rs"))
        .context("read Core configuration editor")?;
    for required in [
        "agena_domain::parse_json_path",
        "agena_domain::get_json_path",
        "agena_runtime::list_json_path",
        "agena_domain::format_json_path",
    ] {
        if !core_edit.contains(required) {
            bail!("Core configuration editor must delegate `{required}` to its owning boundary");
        }
    }
    for forbidden in [
        "pub enum ConfigSettingsLayer",
        "fn push_path_segment(",
        "fn collect_list_entries(",
        "fn scalar_json_value(",
        "fn json_kind(",
    ] {
        if core_edit.contains(forbidden) {
            bail!("Core configuration editor must not retain `{forbidden}`");
        }
    }
    let runtime_settings = fs::read_to_string(
        workspace.join("crates/agena-runtime/src/runtime_config_settings_service.rs"),
    )
    .context("read Runtime configuration settings service")?;
    for required in [
        "pub enum ConfigSettingsLayer",
        "pub(crate) fn get_json_path(",
        "pub(crate) fn format_settings_path",
    ] {
        if !runtime_settings.contains(required) {
            bail!("Runtime configuration settings must retain `{required}`");
        }
    }
    let runtime_overrides =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/config_override.rs"))
            .context("read Runtime configuration override parser")?;
    for required in [
        "pub(crate) enum ConfigOverride",
        "pub(crate) struct LoadConfigRequest",
        "pub(crate) enum RuntimeConfigOverrideError",
        "impl FromStr for ConfigOverride",
    ] {
        if !runtime_overrides.contains(required) {
            bail!("Runtime configuration override parser must retain `{required}`");
        }
    }
    if runtime_overrides.contains("ConfigOverrideArgument") {
        bail!("Runtime configuration override parser must not retain an unused expression wrapper");
    }
    let runtime_composition = fs::read_to_string(
        workspace.join("crates/agena-runtime/src/runtime_composition_config.rs"),
    )
    .context("read Runtime composition input")?;
    for required in [
        "pub(crate) struct RuntimeCompositionConfig",
        "pub(crate) load_request: LoadConfigRequest",
        "pub(crate) database_connection: Option<Arc<DatabaseConnection>>",
        "pub(crate) tracing_reload_handle: Option<TracingFilterReloadHandle>",
    ] {
        if !runtime_composition.contains(required) {
            bail!("Runtime composition input must retain `{required}`");
        }
    }
    let runtime_composition_helpers =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/composition.rs"))
            .context("read Runtime composition helpers")?;
    for required in [
        "pub(crate) struct RuntimeSessionBuildConfig",
        "pub(crate) struct RuntimeSessionManagerConfig",
        "pub(crate) const DEFAULT_MAX_CONCURRENT_TOOLS: usize = 32",
        "pub(crate) fn cache_policy(&self) -> crate::SessionCachePolicy",
        "pub(crate) cache_limits: agena_domain::SessionCacheLimits",
        "pub(crate) max_concurrent_tools: usize",
        "pub(crate) permission: agena_domain::PermissionConfig",
        "pub(crate) tool_presentation: agena_plugin_host::ToolPresentationConfig",
        "pub(crate) fn session_build_config_from_resolved(",
    ] {
        if !runtime_composition_helpers.contains(required) {
            bail!("Runtime session composition values must retain `{required}`");
        }
    }
    let runtime_lib = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
        .context("read Runtime public surface")?;
    if false {
        for required in [
            "pub(crate) use application_services::{\n    RuntimeApplicationServiceCompositionInputs, compose_runtime_application_services,\n};",
            "pub(crate) use bootstrap_result::{",
            "pub(crate) use completion_request::{CompletionRequestInputs, build_completion_request};",
            "pub(crate) use composition::{",
            "pub(crate) use context_governor::ContextGovernor;",
            "pub(crate) use control_state::RuntimeControlState;",
            "pub(crate) use event_bridge::{",
            "pub(crate) use execution_registry::{ExecutionControl, ExecutionControlError, ExecutionRegistry};",
            "pub(crate) use guards::{AbortOnDrop, spawn_abortable, spawn_detached};",
            "pub(crate) use invocation_guard::{InvocationGuard, try_enter_invocation};",
            "pub(crate) use model_catalog_cache::{",
            "pub(crate) use model_catalog_composition::{",
            "pub(crate) use model_catalog_curation::{",
            "pub(crate) use model_catalog_http::{",
            "pub(crate) use model_catalog_live::{",
            "pub(crate) use model_catalog_service::{",
            "pub(crate) use model_catalog_source::{",
            "pub(crate) use monitor::{",
            "pub(crate) use plugin_composition::{",
            "pub(crate) use plugin_config::{dispatch_config_if_nonempty, merge_bundled_plugin_config};",
            "pub(crate) use plugin_shutdown::plugin_shutdown_guard;",
            "pub(crate) use plugin_slot::{current_plugin_host, install_plugin_host};",
            "pub(crate) use process_state::RuntimeProcessState;",
            "pub(crate) use project_paths::{",
            "pub(crate) use provider_composition::{",
            "pub(crate) use provider_model_selection::{",
            "pub(crate) use provider_priorities::provider_model_catalog_priorities;",
            "pub(crate) use registration::{",
            "pub(crate) use optional::build_optional;",
            "pub(crate) use periodic::{run_periodic, wait_for_tick_or_shutdown};",
            "pub(crate) use policy::RuntimeSchedulingPolicy;",
            "pub(crate) use prompt_budget::{",
            "pub(crate) use prompt_merge::merge_system_prompts;",
            "pub(crate) use provider_client_versions::{",
            "pub(crate) use provider_sse::{",
            "pub(crate) use refresh::run_cancellable_refresh;",
            "pub(crate) use refresh_policy::should_refresh;",
            "pub(crate) use reload_gate::ReloadGate;",
            "pub(crate) use reload_watch::run_reload_watch_loop;",
            "pub(crate) use scheduler_composition::compose_scheduler;",
            "pub(crate) use session_cache::{CacheEntry, SessionCache};",
            "pub(crate) use session_cache_policy::SessionCachePolicy;",
            "pub(crate) use session_maintenance::run_session_maintenance;",
            "pub(crate) use snapshot_managed::prune_stale_managed_snapshots;",
            "pub(crate) use snapshot_operations::{",
            "pub(crate) use snapshot_state::RuntimeSnapshotState;",
            "pub(crate) use staleness::is_stale;",
            "pub(crate) use store::{SnapshotStore, TaskControl};",
            "pub(crate) use task_state::RuntimeTaskState;",
            "pub(crate) use tool_output::truncate_tool_output_text;",
            "pub(crate) use usage_stats::{UsageStatRecord, summarize_usage_records};",
            "pub(crate) use watch::{WatchPathStamp, capture_watch_path_stamps, diff_watch_path_stamps};",
            "pub(crate) use watch_paths::{WatchPathSet, runtime_watch_paths};",
            "pub(crate) use compaction_policy::{",
            "pub(crate) use codex_user_agent::{RUNTIME_CODEX_ORIGINATOR, runtime_codex_user_agent};",
            "pub(crate) use connect::connect_or_initialize;",
            "pub(crate) use context_budget::{",
            "pub(crate) use installation_id::resolve_installation_id;",
            "pub(crate) use lsp_config::{",
            "pub(crate) use mcp_runtime::{",
            "pub(crate) use memory::{",
            "pub(crate) use metrics::{",
        ] {
            if !runtime_lib.contains(required) {
                bail!(
                    "Runtime concrete composition internals must remain crate-private: `{required}`"
                );
            }
        }
    }
    for forbidden in [
        "pub use bootstrap_result::{",
        "pub use completion_request::{CompletionRequestInputs, build_completion_request};",
        "pub use composition::{",
        "pub use context_governor::ContextGovernor;",
        "pub use control_state::RuntimeControlState;",
        "pub use event_bridge::{",
        "pub use execution_registry::{ExecutionControl, ExecutionControlError, ExecutionRegistry};",
        "pub use guards::{AbortOnDrop, spawn_abortable, spawn_detached};",
        "pub use invocation_guard::{InvocationGuard, try_enter_invocation};",
        "pub use model_catalog_cache::{",
        "pub use model_catalog_composition::{",
        "pub use model_catalog_curation::{",
        "pub use model_catalog_http::{",
        "pub use model_catalog_live::{",
        "pub use model_catalog_service::{",
        "pub use model_catalog_source::{",
        "pub use monitor::{",
        "pub use plugin_composition::{",
        "pub use plugin_config::{dispatch_config_if_nonempty, merge_bundled_plugin_config};",
        "pub use plugin_shutdown::plugin_shutdown_guard;",
        "pub use plugin_slot::{current_plugin_host, install_plugin_host};",
        "pub use process_state::RuntimeProcessState;",
        "pub use project_paths::{",
        "pub use provider_composition::{",
        "pub use provider_model_selection::{",
        "pub use provider_priorities::provider_model_catalog_priorities;",
        "pub use registration::{",
        "pub use optional::build_optional;",
        "pub use periodic::{run_periodic, wait_for_tick_or_shutdown};",
        "pub use policy::RuntimeSchedulingPolicy;",
        "pub use prompt_budget::{",
        "pub use prompt_merge::merge_system_prompts;",
        "pub use provider_sse::{",
        "pub use refresh::run_cancellable_refresh;",
        "pub use refresh_policy::should_refresh;",
        "pub use reload_gate::ReloadGate;",
        "pub use reload_watch::run_reload_watch_loop;",
        "pub use scheduler_composition::compose_scheduler;",
        "pub use session_cache::{CacheEntry, SessionCache};",
        "pub use session_cache_policy::SessionCachePolicy;",
        "pub use session_maintenance::run_session_maintenance;",
        "pub use snapshot_managed::prune_stale_managed_snapshots;",
        "pub use snapshot_operations::{",
        "pub use snapshot_state::RuntimeSnapshotState;",
        "pub use staleness::is_stale;",
        "pub use store::{SnapshotStore, TaskControl};",
        "pub use task_state::RuntimeTaskState;",
        "pub use tool_output::truncate_tool_output_text;",
        "pub use usage_stats::{UsageStatRecord, summarize_usage_records};",
        "pub use watch::{WatchPathStamp, capture_watch_path_stamps, diff_watch_path_stamps};",
        "pub use watch_paths::{WatchPathSet, runtime_watch_paths};",
        "pub use compaction_policy::{",
        "pub use codex_user_agent::{RUNTIME_CODEX_ORIGINATOR, runtime_codex_user_agent};",
        "pub use connect::connect_or_initialize;",
        "pub use context_budget::{",
        "pub use installation_id::resolve_installation_id;",
        "pub use lsp_config::{",
        "pub use mcp_runtime::{",
        "pub use memory::{",
    ] {
        if runtime_lib.contains(forbidden) {
            bail!(
                "Runtime must not publicly re-export concrete composition internals: `{forbidden}`"
            );
        }
    }
    let core_builder =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/builder.rs"))
            .context("read Core runtime builder")?;
    if core_builder.contains("pub struct AgenaRuntimeConfig")
        || !core_builder.contains("pub(crate) async fn new(config: RuntimeCompositionConfig)")
        || !core_builder.contains("pub(crate) struct AgenaRuntime")
        || core_builder.contains("pub struct AgenaRuntime")
    {
        bail!("Core runtime must consume, not define, the Runtime composition input");
    }
    let core_runtime_module =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/mod.rs"))
            .context("read Core runtime module")?;
    if core_runtime_module.contains("AgenaRuntimeConfig") {
        bail!("Core runtime module must not re-export the deleted Core composition input");
    }
    if core_runtime_module.contains("pub use agena_runtime::TracingFilterReloadHandle") {
        bail!("Core runtime module must not publicly re-export Runtime tracing values");
    }
    let core_snapshot_builders =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/snapshot/builders.rs"))
            .context("read Core snapshot builders")?;
    if core_snapshot_builders.contains("pub(super) struct SessionBuildConfig") {
        bail!("Core snapshot builders must not retain Runtime session composition values");
    }
    let core_snapshot_module =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/snapshot/mod.rs"))
            .context("read Core snapshot session composition")?;
    if !core_snapshot_module
        .contains("agena_runtime::session_build_config_from_resolved(&resolution.config)")
        || !core_snapshot_module.contains("pub(crate) struct RuntimeSnapshot")
        || core_snapshot_module.contains("pub struct RuntimeSnapshot")
    {
        bail!("Core snapshot must consume Runtime session policy projection");
    }
    let runtime_usage_stats =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/usage_stats.rs"))
            .context("read Runtime usage-stat record")?;
    if !runtime_usage_stats.contains("pub struct UsageStatRecord")
        || !runtime_usage_stats.contains("pub fn summarize_usage_records")
    {
        bail!("Runtime must own usage-stat records and their schema-neutral aggregation");
    }
    let runtime_context_budget =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/context_budget.rs"))
            .context("read Runtime context-budget policy")?;
    if !runtime_context_budget.contains("pub fn estimate_prompt_budget_threshold_tokens") {
        bail!("Runtime must own the prompt-budget threshold policy");
    }
    let runtime_prompt_budget =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/prompt_budget.rs"))
            .context("read Runtime prompt-budget policy")?;
    if !runtime_prompt_budget.contains("pub fn estimate_prompt_tokens_from_chars") {
        bail!("Runtime must own prompt-character token estimation");
    }
    for required in [
        "pub const APPROX_CHARS_PER_TOKEN",
        "pub const MIN_PROMPT_BUDGET_TOKENS",
    ] {
        if !runtime_prompt_budget.contains(required) {
            bail!("Runtime must own shared prompt-budget constant `{required}`");
        }
    }
    let core_session_module =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/session/mod.rs"))
            .context("read Core session module")?;
    if core_session_module.contains("pub fn estimate_prompt_budget_threshold_tokens") {
        bail!("Core session module must not retain the Runtime prompt-budget threshold policy");
    }
    let core_prompt_window =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/session/prompt_window.rs"))
            .context("read Core prompt window")?;
    if core_prompt_window.contains("fn approximate_tokens_from_chars") {
        bail!("Core prompt window must not retain Runtime token estimation");
    }
    for forbidden in [
        "const APPROX_CHARS_PER_TOKEN",
        "const MIN_PROMPT_BUDGET_TOKENS",
    ] {
        if core_prompt_window.contains(forbidden) {
            bail!(
                "Core prompt window must not retain Runtime prompt-budget constant `{forbidden}`"
            );
        }
    }
    let core_raw_config =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/config/raw.rs"))
            .context("read Core raw configuration parser")?;
    if !core_raw_config.contains("crate::agent::validate_permission_config(permission)")
        || core_raw_config.contains("crate::agent::Agent::new(")
    {
        bail!(
            "Core raw configuration must use the narrow permission-validation adapter rather than construct Agent"
        );
    }
    if !core_raw_config.contains("agena_runtime::merge_bundled_plugin_config")
        || core_raw_config.contains("resolve_plugin_config")
    {
        bail!("Core raw configuration must delegate bundled-plugin merge precedence to Runtime");
    }
    if core_raw_config.contains("fn parse_bool") {
        bail!("Core raw configuration must not retain the Runtime-owned boolean parser");
    }
    if core_raw_config.contains("fn normalize_optional")
        || core_raw_config.contains("fn normalize_optional_string")
    {
        bail!("Core raw configuration must not retain Runtime-owned optional-string normalization");
    }
    if !core_raw_config.contains("agena_runtime::read_config_json")
        || !core_raw_config.contains("agena_runtime::parse_config_json")
        || core_raw_config.contains("fn parse_config_value")
    {
        bail!("Core raw configuration must use Runtime-owned JSON document parsing");
    }
    let runtime_plugin_config =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/plugin_config.rs"))
            .context("read Runtime plugin configuration policy")?;
    assert!(runtime_plugin_config.contains("pub fn merge_bundled_plugin_config"));
    let runtime_config_error =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/config_error.rs"))
            .context("read Runtime configuration error")?;
    assert!(runtime_config_error.contains("pub(crate) enum ConfigError"));
    assert!(runtime_config_error.contains("apply_config_env_number"));
    assert!(runtime_config_error.contains("parse_config_bool"));
    assert!(runtime_config_error.contains("merge_optional_config"));
    assert!(runtime_config_error.contains("normalize_config_optional"));
    assert!(runtime_config_error.contains("read_config_json"));
    assert!(runtime_config_error.contains("parse_config_json"));
    assert!(runtime_config_error.contains("reject_unsupported_mode_environment"));
    let runtime_overrides =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/config_override.rs"))
            .context("read Runtime override parser")?;
    assert!(runtime_overrides.contains("pub(crate) fn parse_config_override_expressions"));
    let runtime_bootstrap_request =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/bootstrap_request.rs"))
            .context("read Runtime bootstrap request adapter")?;
    assert!(
        runtime_bootstrap_request.contains("pub(crate) fn load_config_request_from_bootstrap")
            && !runtime_bootstrap_request.contains("pub fn load_config_request_from_bootstrap")
    );
    assert!(runtime_bootstrap_request.contains("crate::read_config_json"));
    assert!(!runtime_bootstrap_request.contains("std::fs::read_to_string"));
    assert!(!runtime_bootstrap_request.contains("serde_json::from_str"));
    assert!(!runtime_bootstrap_request.contains("std::env::var"));
    let runtime_config_values =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/config_values/resolved.rs"))
            .context("read Runtime configuration-resolution metadata")?;
    assert!(runtime_config_values.contains("pub fn from_layer_presence"));
    let core_config_loader =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/config/loader.rs"))
            .context("read Core configuration loader")?;
    assert!(!core_config_loader.contains("let mut applied_layers"));
    assert!(!core_config_loader.contains("AGENA_MODE"));
    assert!(runtime_config_error.contains("config_error_to_settings_error"));
    assert!(runtime_config_error.contains("settings_error_to_config_error"));
    assert!(!runtime_config_error.contains("AppError"));
    let core_error = fs::read_to_string(workspace.join("crates/agena-runtime/src/error.rs"))
        .context("read Core application error boundary")?;
    assert!(core_error.contains("agena_runtime::ConfigError"));
    assert!(!core_error.contains("crate::config::ConfigError"));
    let core_config_edit =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/config/edit.rs"))
            .context("read Core configuration editor")?;
    assert!(!core_config_edit.contains("fn map_settings_error"));
    assert!(!core_config_edit.contains("fn settings_layer_path"));
    let runtime_config_settings = fs::read_to_string(
        workspace.join("crates/agena-runtime/src/runtime_config_settings_service.rs"),
    )
    .context("read Runtime config-settings service")?;
    assert!(runtime_config_settings.contains("pub fn config_settings_layer_path"));
    let core_runtime_builder =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/builder.rs"))
            .context("read Core runtime builder")?;
    assert!(!core_runtime_builder.contains("fn runtime_config_settings_error"));
    assert!(!core_runtime_builder.contains("env::current_dir"));
    assert!(!core_runtime_builder.contains("expression.parse()"));
    assert!(!core_runtime_builder.contains("LoadConfigRequest {"));
    assert!(
        core_runtime_builder.contains("let initial_resolution = bootstrap_preflight")
            && core_runtime_builder.contains(".is_none()")
    );
    let runtime_composition_config = fs::read_to_string(
        workspace.join("crates/agena-runtime/src/runtime_composition_config.rs"),
    )
    .context("read Runtime composition configuration")?;
    assert!(runtime_composition_config.contains("pub(crate) fn resolve_workspace_root"));
    assert!(runtime_composition_config.contains("bootstrap_preflight"));
    assert!(runtime_composition_config.contains("preflight.workspace_root.clone()"));
    let core_session_cost =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/session/cost.rs"))
            .context("read Core session cost reducer")?;
    if core_session_cost.contains("pub struct UsageStatRecord") {
        bail!("Core session-cost reducer must not retain the usage-stat record value");
    }
    if core_session_cost.contains("summarize_usage_records") {
        bail!("Core session-cost reducer must not retain Runtime usage-stat aggregation");
    }
    if core_session_cost.contains("fn estimate_model_token_rates")
        || core_session_cost.contains("const PRICING_PREFIXES")
    {
        bail!("Core session-cost reducer must not retain provider pricing policy");
    }
    let provider_usage_cost =
        fs::read_to_string(workspace.join("crates/agena-provider/src/usage_cost.rs"))
            .context("read Provider usage-cost policy")?;
    for required in [
        "pub fn estimate_completion_usage_cost_usd",
        "pub fn completion_usage_cost_contribution",
        "const PRICING_PREFIXES",
        "fn normalize_model_id",
    ] {
        if !provider_usage_cost.contains(required) {
            bail!("Provider usage-cost policy must retain `{required}`");
        }
    }
    let core_message_module =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/message/mod.rs"))
            .context("read Core message module")?;
    if core_message_module.contains("MessageUsage")
        || workspace
            .join("crates/agena-runtime/src/message/usage.rs")
            .exists()
    {
        bail!("Core message module must not retain a CompletionUsage compatibility facade");
    }
    let core_overrides =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/config/overrides.rs"))
            .context("read Core configuration override adapter")?;
    for forbidden in [
        "pub enum ConfigOverride",
        "pub struct ConfigOverrideArgument",
        "pub struct LoadConfigRequest",
        "impl FromStr for ConfigOverride",
        "fn parse_provider_override",
        "fn parse_agent_override",
    ] {
        if core_overrides.contains(forbidden) {
            bail!("Core configuration override adapter must not retain `{forbidden}`");
        }
    }
    if !core_overrides.contains("pub(crate) fn apply_config_override") {
        bail!("Core configuration override adapter must retain raw-schema application");
    }
    Ok(())
}

fn assert_domain_agent_profile_values_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let domain_values = workspace.join("crates/agena-domain/src/agent_selection.rs");
    let domain_source = fs::read_to_string(&domain_values)
        .with_context(|| format!("read {}", domain_values.display()))?;
    for required in [
        "pub struct AgentSelectionConfig",
        "pub struct AgentToolsConfig",
    ] {
        if !domain_source.contains(required) {
            bail!("domain agent-profile values must retain `{required}`");
        }
    }
    let core_agents = fs::read_to_string(workspace.join("crates/agena-runtime/src/agents/mod.rs"))
        .context("read Core agent registry for value ownership")?;
    for forbidden in [
        "pub struct AgentSelectionConfig",
        "pub struct AgentToolsConfig",
    ] {
        if core_agents.contains(forbidden) {
            bail!("Core agent registry must not define `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_runtime_memory_plugin_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime_memory = workspace.join("crates/agena-runtime/src/memory/mod.rs");
    let runtime_source = fs::read_to_string(&runtime_memory)
        .with_context(|| format!("read {}", runtime_memory.display()))?;
    for required in ["MemoryPlugin", "new_memory_plugin", "MEMORY_PLUGIN_ID"] {
        if !runtime_source.contains(required) {
            bail!("Runtime memory module must retain `{required}`");
        }
    }
    Ok(())
}

fn assert_runtime_web_plugin_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime_web = workspace.join("crates/agena-runtime/src/web/mod.rs");
    let runtime_source = fs::read_to_string(&runtime_web)
        .with_context(|| format!("read {}", runtime_web.display()))?;
    for required in ["WebPlugin", "new_web_plugin", "WEB_PLUGIN_ID"] {
        if !runtime_source.contains(required) {
            bail!("Runtime web-plugin module must retain `{required}`");
        }
    }
    let web_manifest = fs::read_to_string(workspace.join("crates/agena-web/Cargo.toml"))
        .context("read web manifest for fetch-coordinator ownership")?;
    for required in [
        "governor = { workspace = true }",
        "moka = { workspace = true }",
    ] {
        if !web_manifest.contains(required) {
            bail!("web package must own fetch-coordinator dependency `{required}`");
        }
    }
    let coordinator =
        fs::read_to_string(workspace.join("crates/agena-web/src/fetch_coordinator.rs"))
            .context("read web fetch coordinator")?;
    for required in [
        "pub struct WebFetchCoordinator",
        "pub struct WebFetchCoordinatorConfig",
        "pub async fn wait_for_url_host",
        "pub async fn fetch_or_cached",
    ] {
        if !coordinator.contains(required) {
            bail!("web fetch coordinator must retain `{required}`");
        }
    }
    let runtime_manifest = fs::read_to_string(workspace.join("crates/agena-runtime/Cargo.toml"))
        .context("read Runtime manifest for web cache ownership")?;
    if runtime_manifest.contains("moka =") {
        bail!("Runtime must not retain the web fetch cache dependency");
    }
    let plugin_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/web/plugin.rs"))
            .context("read Runtime web plugin")?;
    for forbidden in ["moka::", "build_host_limiter", "fetch_cache_key"] {
        if plugin_source.contains(forbidden) {
            bail!(
                "Runtime web plugin must consume the web fetch coordinator instead of `{forbidden}`"
            );
        }
    }
    if !plugin_source.contains("WebFetchCoordinator")
        || !plugin_source.contains("fetch_or_cached")
        || !plugin_source.contains("wait_for_url_host")
    {
        bail!(
            "Runtime web plugin must retain the permission/transport adapter over web coordination"
        );
    }
    Ok(())
}

fn assert_domain_permission_decision_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let domain_resolution = workspace.join("crates/agena-domain/src/permission_resolution.rs");
    let domain_source = fs::read_to_string(&domain_resolution)
        .with_context(|| format!("read {}", domain_resolution.display()))?;
    if !domain_source.contains("pub fn decide_from_mode") {
        bail!("domain permission resolution must own decide_from_mode");
    }
    if workspace
        .join("crates/agena-runtime/src/permission/store.rs")
        .exists()
    {
        bail!("legacy Core permission decision store must not return");
    }
    Ok(())
}

fn assert_tool_search_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tool_search = workspace.join("crates/agena-tool/src/tool_search.rs");
    let tool_source = fs::read_to_string(&tool_search)
        .with_context(|| format!("read {}", tool_search.display()))?;
    for required in [
        "pub struct ToolSearchDocument",
        "pub fn search_tools",
        "NGRAM_TOKENIZER",
    ] {
        if !tool_source.contains(required) {
            bail!("tool search implementation must retain `{required}`");
        }
    }
    if workspace.join("crates/agena-runtime/src/search").exists() {
        bail!("legacy Core search module must not return");
    }
    let core_root = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
        .context("read legacy Core root for search ownership")?;
    if core_root.contains("mod search") {
        bail!("legacy Core root must not expose search implementation");
    }
    Ok(())
}

fn assert_tool_code_search_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tool_manifest = fs::read_to_string(workspace.join("crates/agena-tool/Cargo.toml"))
        .context("read tool manifest for structural code-search ownership")?;
    for required in [
        "ast-grep-core = { workspace = true }",
        "ast-grep-language = { workspace = true }",
        "tree-sitter = { workspace = true }",
        "walkdir = { workspace = true }",
    ] {
        if !tool_manifest.contains(required) {
            bail!("tool package must own structural code-search dependency `{required}`");
        }
    }
    let tool_source = fs::read_to_string(workspace.join("crates/agena-tool/src/code_search.rs"))
        .context("read tool structural code-search implementation")?;
    for required in [
        "pub enum CodeLanguage",
        "pub fn search_ast",
        "pub fn syntax_tree",
        "ast_grep",
        "tree_sitter",
    ] {
        if !tool_source.contains(required) {
            bail!("tool structural code-search module must retain `{required}`");
        }
    }
    let runtime_manifest = fs::read_to_string(workspace.join("crates/agena-runtime/Cargo.toml"))
        .context("read Runtime manifest for structural code-search ownership")?;
    for forbidden in ["ast-grep-core =", "ast-grep-language =", "tree-sitter ="] {
        if runtime_manifest.contains(forbidden) {
            bail!("Runtime must not retain direct structural code-search dependency `{forbidden}`");
        }
    }
    let runtime_source =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/plugins/provided/code.rs"))
            .context("read Runtime code plugin adapter")?;
    for forbidden in ["ast_grep_", "tree_sitter::", "Pattern::try_new"] {
        if runtime_source.contains(forbidden) {
            bail!("Runtime code plugin must consume the tool search algorithm directly");
        }
    }
    if !runtime_source.contains("StructuralSearchRequest")
        || !runtime_source.contains("SyntaxTreeRequest")
        || !runtime_source.contains("search_ast(")
        || !runtime_source.contains("syntax_tree(")
    {
        bail!("Runtime code plugin must remain the SDK/output adapter over agena-tool search");
    }
    Ok(())
}

fn assert_tool_shell_contract_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tool_shell = workspace.join("crates/agena-tool/src/shell.rs");
    let tool_source = fs::read_to_string(&tool_shell)
        .with_context(|| format!("read {}", tool_shell.display()))?;
    for required in [
        "pub const DEFAULT_SHELL_TIMEOUT_MS",
        "pub fn truncate_shell_output",
        "pub fn shell_command_for_platform",
        "pub fn powershell_command_for_windows",
    ] {
        if !tool_source.contains(required) {
            bail!("tool shell contract must retain `{required}`");
        }
    }
    let tool_analysis =
        fs::read_to_string(workspace.join("crates/agena-tool/src/shell_analysis.rs"))
            .context("read tool shell-analysis values")?;
    for required in [
        "pub struct CommandAnalysis",
        "pub enum CommandClassification",
        "pub enum ExitInterpretation",
        "pub fn interpret_exit_code",
        "pub fn shell_tokens",
        "pub fn command_segments",
        "pub fn first_command",
        "pub fn contains_write_redirection",
        "pub fn contains_input_redirection",
        "pub fn network_command_reason",
        "pub fn analyze_command",
        "pub fn mutating_command_reason",
        "pub fn filesystem_command_reason",
        "pub fn curl_cookie_option_uses_file",
        "pub fn curl_data_option_uses_file",
        "pub fn curl_form_option_uses_file",
        "pub fn powershell_web_cmdlet_filesystem_reason",
    ] {
        if !tool_analysis.contains(required) {
            bail!("tool shell-analysis values must retain `{required}`");
        }
    }
    let core_shell =
        fs::read_to_string(workspace.join("crates/agena-runtime/src/tool/shell_tools.rs"))
            .context("read Core shell tools")?;
    if !core_shell.contains("agena_tool::shell_analysis::filesystem_command_reason(command)") {
        bail!("Core filesystem-effect validation must delegate command analysis to Tool");
    }
    for forbidden in [
        "pub(crate) const DEFAULT_TIMEOUT_MS",
        "pub(crate) fn truncate_output",
        "pub(crate) fn shell_command_for_platform",
        "pub(crate) fn powershell_command_for_windows",
        "pub(crate) struct CommandAnalysis",
        "pub(crate) enum CommandClassification",
        "pub(crate) enum ExitInterpretation",
        "pub(crate) fn interpret_exit_code",
        "fn shell_tokens",
        "fn command_segments",
        "fn first_command",
        "fn contains_write_redirection",
        "fn contains_input_redirection",
        "pub(crate) fn network_command_reason",
        "fn network_segment_reason",
        "pub(crate) fn analyze_command",
        "fn classify_command",
        "fn classify_segment",
        "fn is_known_read_only_command",
        "fn is_obvious_write_command",
        "fn is_in_place_flag",
        "pub(crate) fn mutating_command_reason",
        "fn filesystem_command_reason",
        "fn filesystem_segment_reason",
        "fn curl_filesystem_reason",
        "fn curl_cookie_option_uses_file",
        "fn curl_data_option_uses_file",
        "fn curl_form_option_uses_file",
        "fn powershell_web_cmdlet_filesystem_reason",
    ] {
        if core_shell.contains(forbidden) {
            bail!("Core shell tools must not retain `{forbidden}`");
        }
    }
    Ok(())
}

fn assert_runtime_installation_id_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime_id = workspace.join("crates/agena-runtime/src/installation_id.rs");
    let runtime_source = fs::read_to_string(&runtime_id)
        .with_context(|| format!("read {}", runtime_id.display()))?;
    for required in ["pub async fn resolve_installation_id", "Uuid::new_v4"] {
        if !runtime_source.contains(required) {
            bail!("Runtime installation-id module must retain `{required}`");
        }
    }
    Ok(())
}

fn assert_runtime_project_path_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime_paths = workspace.join("crates/agena-runtime/src/project_paths.rs");
    let runtime_source = fs::read_to_string(&runtime_paths)
        .with_context(|| format!("read {}", runtime_paths.display()))?;
    for required in [
        "pub fn project_state_dir",
        "pub fn generated_image_artifact_path",
        "GENERATED_IMAGE_ARTIFACTS_DIR",
    ] {
        if !runtime_source.contains(required) {
            bail!("Runtime project-path module must retain `{required}`");
        }
    }
    Ok(())
}

fn assert_domain_execution_selection_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let domain_selection = workspace.join("crates/agena-domain/src/execution_selection.rs");
    let domain_source = fs::read_to_string(&domain_selection)
        .with_context(|| format!("read {}", domain_selection.display()))?;
    for required in [
        "pub struct ExecutionSelection",
        "PermissionConfig",
        "ModelRef",
    ] {
        if !domain_source.contains(required) {
            bail!("domain execution selection must retain `{required}`");
        }
    }
    if domain_source.contains("sea_orm") || domain_source.contains("FromJsonQueryResult") {
        bail!("domain execution selection must not carry persistence derives");
    }

    let core_selection = workspace.join("crates/agena-runtime/src/execution_prefs.rs");
    if core_selection.exists() {
        bail!("legacy Core execution_prefs module must not return");
    }
    let core_root = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
        .context("read legacy Core root for execution selection ownership")?;
    if core_root.contains("execution_prefs") {
        bail!("legacy Core root must not expose execution_prefs");
    }
    Ok(())
}

fn assert_tui_presentation_asset_ownership() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let app_source = workspace.join("apps/agena/src");
    if app_source.join("i18n.rs").exists() {
        bail!("final app must not own the TUI i18n implementation");
    }
    if workspace.join("apps/agena/locales").exists() {
        bail!("final app must not own TUI locale assets");
    }

    let tui_i18n = workspace.join("crates/agena-tui/src/i18n.rs");
    let tui_i18n_source =
        fs::read_to_string(&tui_i18n).with_context(|| format!("read {}", tui_i18n.display()))?;
    for required in [
        "static_loader!",
        "SUPPORTED_LOCALES",
        "macro_rules! fl_args",
    ] {
        if !tui_i18n_source.contains(required) {
            bail!("TUI i18n implementation must retain `{required}`");
        }
    }
    let locale_count = fs::read_dir(workspace.join("crates/agena-tui/locales"))
        .context("read TUI locale assets")?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .count();
    if locale_count != 8 {
        bail!("expected exactly 8 TUI locale directories, found {locale_count}");
    }

    for source in collect_rust_sources(&app_source)? {
        if source.contains("crate::fl_args!") || source.contains("crate::i18n") {
            bail!("final app must import TUI i18n values and macro directly");
        }
    }

    let tui_capabilities = workspace.join("crates/agena-tui/src/terminal_capabilities.rs");
    let tui_capabilities_source = fs::read_to_string(&tui_capabilities)
        .with_context(|| format!("read {}", tui_capabilities.display()))?;
    for required in [
        "pub struct CapabilityEvidence",
        "pub enum CapabilityPath",
        "pub enum ProviderReadiness",
        "pub struct TerminalCapabilities",
        "pub struct TerminalDiagnostic",
        "fn lifecycle_capabilities",
    ] {
        if !tui_capabilities_source.contains(required) {
            bail!("TUI terminal-capability slice must retain `{required}`");
        }
    }
    let app_capabilities = fs::read_to_string(app_source.join("terminal/capabilities.rs"))
        .context("read final-app terminal capability composition")?;
    for forbidden in [
        "pub enum Support",
        "pub enum CapabilitySource",
        "pub struct CapabilityEvidence",
        "pub enum CapabilityPath",
        "pub enum ProviderReadiness",
        "pub struct TerminalCapabilities",
        "pub struct TerminalDiagnostic",
        "impl TerminalCapabilities",
    ] {
        if app_capabilities.contains(forbidden) {
            bail!(
                "final app must consume TUI terminal capability values, not define `{forbidden}`"
            );
        }
    }
    Ok(())
}

fn assert_build_artifact_policy() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = fs::read_to_string(workspace.join("Cargo.toml"))
        .context("read workspace manifest for build-artifact policy")?;
    for required in [
        "[profile.dev]\n",
        "[profile.test]\n",
        "debug = 0\n",
        "incremental = true\n",
    ] {
        if !manifest.contains(required) {
            bail!("workspace build profiles must retain storage guard `{required}`");
        }
    }

    let bounded_runner = fs::read_to_string(workspace.join("scripts/cargo-bounded.sh"))
        .context("read bounded Cargo runner")?;
    // Broad workspace/feature verification intentionally avoids accumulating
    // many incompatible incremental caches; the normal Cargo edit loop keeps
    // profile-level incremental compilation enabled above.
    for required in ["AGENA_MAX_TARGET_GIB", "CARGO_INCREMENTAL=0", "du -sk"] {
        if !bounded_runner.contains(required) {
            bail!("bounded Cargo runner must retain target-size guard `{required}`");
        }
    }
    let timing_probe = fs::read_to_string(workspace.join("scripts/check-build-timings.sh"))
        .context("read build timing probe")?;
    for required in [
        "check_retained_target_size",
        "AGENA_MAX_TARGET_GIB",
        "assert_tui_leaf_rebuild_attribution",
        "agena_provider_bedrock_streaming",
        "agena_storage_sqlite",
        "agena_api_server",
        "agena_client",
        "cargo check -p agena-tui --locked -vv",
        "cargo build -p agena --locked -vv",
    ] {
        if !timing_probe.contains(required) {
            bail!("build timing probe must retain graph/target guard `{required}`");
        }
    }
    let cleanup_sources = [
        (
            "crates/agena-runtime/src/model_catalog/decorate.rs",
            "apply_catalog_display_name_as_fallback",
        ),
        (
            "crates/agena-runtime/src/session/manager/replies.rs",
            "join_runtime_context_lines",
        ),
        (
            "crates/agena-runtime/src/provider/gemini.rs",
            "build_gemini_provider_native_tools_only",
        ),
        (
            "crates/agena-runtime/src/provider/ollama.rs",
            "completion_response_stream",
        ),
        (
            "crates/agena-runtime/src/provider/openai/openai_response_builders.rs",
            "completion_response_stream",
        ),
        (
            "crates/agena-runtime/src/provider/anthropic/anthropic_transport.rs",
            "completion_response_stream",
        ),
        (
            "crates/agena-runtime/src/provider/gemini/gemini_adapter.rs",
            "completion_response_stream",
        ),
        (
            "crates/agena-runtime/src/provider/amazon_bedrock/bedrock_adapter.rs",
            "completion_response_stream",
        ),
        (
            "crates/agena-runtime/src/provider/mod.rs",
            "#[allow(dead_code)]",
        ),
        (
            "apps/agena/src/app/plugin_workbench.rs",
            "#[allow(dead_code)]",
        ),
        (
            "crates/agena-macros/src/input_arg_support.rs",
            "#[allow(dead_code)]",
        ),
    ];
    for (path, forbidden) in cleanup_sources {
        let source = fs::read_to_string(workspace.join(path))
            .with_context(|| format!("read final-cleanup source {path}"))?;
        if source.contains(forbidden) {
            bail!("final cleanup must not restore `{forbidden}` in {path}");
        }
    }
    for path in ["README.md", "docs/configuration.md"] {
        let source = fs::read_to_string(workspace.join(path))
            .with_context(|| format!("read active developer documentation {path}"))?;
        if source.contains("crates/agena/src/") {
            bail!(
                "active developer documentation {path} must not reference deleted crates/agena source"
            );
        }
    }
    Ok(())
}

fn assert_default_member_boundary(packages: &BTreeMap<&str, &Package>) -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = fs::read_to_string(workspace.join("Cargo.toml"))
        .context("read workspace manifest for default-member boundary")?;
    if !manifest.contains("default-members = [\"apps/agena\"]") {
        bail!("workspace default-members must contain only `apps/agena`");
    }
    let app = packages
        .get("agena")
        .context("final terminal app package must be present")?;
    if !app.manifest_path.ends_with("apps/agena/Cargo.toml") {
        bail!("package `agena` must resolve to apps/agena");
    }
    Ok(())
}

fn assert_terminal_binary(packages: &BTreeMap<&str, &Package>) -> Result<()> {
    let agena_bins = packages
        .values()
        .flat_map(|package| package.targets.iter())
        .filter(|target| target.name == "agena" && target.kind.iter().any(|kind| kind == "bin"))
        .count();
    if agena_bins != 1 {
        bail!("expected exactly one production binary named `agena`, found {agena_bins}");
    }

    let legacy_bins = packages
        .values()
        .flat_map(|package| package.targets.iter())
        .filter(|target| target.name == "agena-tui" && target.kind.iter().any(|kind| kind == "bin"))
        .count();
    if legacy_bins != 0 {
        bail!("obsolete `agena-tui` binary target must not exist");
    }
    Ok(())
}

fn assert_legacy_monolith_deleted(packages: &BTreeMap<&str, &Package>) -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if workspace.join("crates/agena/Cargo.toml").exists() || packages.contains_key("agena-core") {
        bail!("the legacy crates/agena monolith must be deleted, not retained as a facade");
    }
    let manifest = fs::read_to_string(workspace.join("Cargo.toml"))
        .context("read workspace manifest for legacy Core alias")?;
    if manifest.contains("package = \"agena-core\"") || manifest.contains("path = \"crates/agena\"")
    {
        bail!("workspace manifest must not retain the legacy Core package alias");
    }
    Ok(())
}

fn assert_forbidden_edges(packages: &BTreeMap<&str, &Package>) -> Result<()> {
    for &(from, to) in FORBIDDEN_EDGES {
        let Some(package) = packages.get(from) else {
            continue;
        };
        if package
            .dependencies
            .iter()
            .any(|dependency| dependency.name == to && dependency.kind.as_deref() != Some("dev"))
        {
            bail!("forbidden dependency: package `{from}` must not depend on `{to}`");
        }
    }
    Ok(())
}

/// Rust modules must be connected through the module system, not by injecting
/// source text. Apart from making ownership and incremental compilation less
/// obvious, `include!`-style composition hides dependency boundaries while a
/// refactor is in progress. Keep this repository-wide invariant in the same
/// executable check as the Cargo graph rules.
fn assert_no_textual_source_includes(packages: &BTreeMap<&str, &Package>) -> Result<()> {
    for package in packages.values() {
        let package_root = package
            .manifest_path
            .parent()
            .context("package manifest has no parent directory")?;
        assert_directory_has_no_textual_source_includes(package_root)?;
    }
    Ok(())
}

fn assert_directory_has_no_textual_source_includes(directory: &Path) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("failed to read {}", directory.display()))?
    {
        let entry = entry.with_context(|| format!("failed to inspect {}", directory.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if file_type.is_dir() {
            assert_directory_has_no_textual_source_includes(&path)?;
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        for macro_name in ["include!", "include_str!", "include_bytes!"] {
            if contains_textual_source_include(&source, macro_name) {
                bail!(
                    "textual source inclusion `{macro_name}` is forbidden: {}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

fn collect_rust_sources(directory: &Path) -> Result<Vec<String>> {
    let mut sources = Vec::new();
    for entry in fs::read_dir(directory)
        .with_context(|| format!("failed to read {}", directory.display()))?
    {
        let entry = entry.with_context(|| format!("failed to inspect {}", directory.display()))?;
        let path = entry.path();
        if path.is_dir() {
            sources.extend(collect_rust_sources(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(
                fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?,
            );
        }
    }
    Ok(sources)
}

fn contains_textual_source_include(source: &str, macro_name: &str) -> bool {
    source.lines().any(|line| {
        let code = line.split_once("//").map_or(line, |(code, _)| code);
        code.match_indices(macro_name).any(|(offset, _)| {
            // The checker necessarily contains the macro names in string
            // literals. Ignore matches inside ordinary quoted strings while
            // still flagging real Rust macro invocations.
            let before = &code[..offset];
            !before
                .chars()
                .fold(
                    (false, false),
                    |(inside, escaped), character| match character {
                        '\\' if !escaped => (inside, true),
                        '"' if !escaped => (!inside, false),
                        _ => (inside, false),
                    },
                )
                .0
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn future_contract_has_no_duplicate_rules() {
        let rules = FORBIDDEN_EDGES
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(rules.len(), FORBIDDEN_EDGES.len());
    }

    #[test]
    fn contracts_cannot_depend_on_the_final_application_package() {
        let rules = FORBIDDEN_EDGES
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();

        for contract in [
            "agena-provider",
            "agena-tool",
            "agena-storage",
            "agena-api",
            "agena-client",
        ] {
            assert!(
                rules.contains(&(contract, "agena")),
                "{contract} must not depend on the final application package"
            );
        }
    }

    #[test]
    fn default_member_boundary_is_terminal_app_only() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = fs::read_to_string(workspace.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("default-members = [\"apps/agena\"]"));
    }

    #[test]
    fn remote_client_is_transport_and_api_contract_only() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = fs::read_to_string(workspace.join("crates/agena-client/Cargo.toml"))
            .expect("read remote client manifest");
        for forbidden in [
            "agena-core",
            "agena =",
            "agena-domain",
            "agena-runtime",
            "agena-application",
            "sea-orm",
            "ratatui",
            "clap",
        ] {
            assert!(
                !manifest.contains(forbidden),
                "remote client must not depend on {forbidden}"
            );
        }
        for source in collect_rust_sources(&workspace.join("crates/agena-client/src"))
            .expect("read remote client sources")
        {
            assert!(!source.contains("agena::"));
            assert!(!source.contains("agena_application"));
            assert!(!source.contains("agena_runtime"));
        }
        let health_fixture =
            workspace.join("crates/agena-client/tests/fixtures/health-response.json");
        let error_fixture =
            workspace.join("crates/agena-client/tests/fixtures/api-error-not-found.json");
        let ws_hello_fixture = workspace.join("crates/agena-client/tests/fixtures/ws-hello.json");
        let ws_pong_fixture = workspace.join("crates/agena-client/tests/fixtures/ws-pong.json");
        let ws_error_fixture = workspace.join("crates/agena-client/tests/fixtures/ws-error.json");
        assert!(health_fixture.is_file(), "missing health protocol fixture");
        assert!(
            error_fixture.is_file(),
            "missing API error protocol fixture"
        );
        for fixture in [&ws_hello_fixture, &ws_pong_fixture, &ws_error_fixture] {
            assert!(fixture.is_file(), "missing WebSocket protocol fixture");
        }
        serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(health_fixture).expect("read health protocol fixture"),
        )
        .expect("health protocol fixture must be valid JSON");
        serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(error_fixture).expect("read API error protocol fixture"),
        )
        .expect("API error protocol fixture must be valid JSON");
        for fixture in [&ws_hello_fixture, &ws_pong_fixture, &ws_error_fixture] {
            serde_json::from_str::<serde_json::Value>(
                &fs::read_to_string(fixture).expect("read WebSocket protocol fixture"),
            )
            .expect("WebSocket protocol fixture must be valid JSON");
        }
        let server = fs::read_to_string(workspace.join("crates/agena-api-server/src/lib.rs"))
            .expect("read API server router");
        assert!(server.contains(".route(\"/api/v1/health\", get(rest::health))"));
        assert!(server.contains("health_route_is_served_by_the_real_api_router"));
        assert!(server.contains("websocket_upgrade_serves_shared_hello_and_pong_frames"));
        assert!(server.contains("encode websocket health query"));
        assert!(server.contains("encode websocket create workspace command"));
        assert!(server.contains("encode websocket delete workspace command"));
        assert!(server.contains("encode websocket subscribe request"));
        assert!(server.contains("encode websocket unsubscribe request"));
        assert!(server.contains("publish websocket notification fixture event"));
        assert!(server.contains("/api/v1/runtime"));
        assert!(server.contains("RuntimeStatusResponse"));
        assert!(server.contains("health_route_is_served_by_the_real_api_router"));
        assert!(server.contains("/api/v1/workspaces"));
        assert!(server.contains("create workspace request"));
        assert!(server.contains("agena_api::ApiError"));
        assert!(server.contains("ErrorCode::NotFound"));
        let server_ws = fs::read_to_string(workspace.join("crates/agena-api-server/src/ws.rs"))
            .expect("read API server WebSocket transport");
        assert!(server_ws.contains("ServerMessage::Hello"));
        assert!(server_ws.contains("ServerMessage::Pong"));
        let client_http = fs::read_to_string(workspace.join("crates/agena-client/src/http.rs"))
            .expect("read remote client HTTP transport");
        assert!(client_http.contains("pub async fn health"));
        assert!(client_http.contains("/api/v1/health"));
    }

    #[test]
    fn tui_owns_action_and_terminal_state_contracts() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let transcript = fs::read_to_string(workspace.join("crates/agena-tui/src/transcript.rs"))
            .expect("read TUI transcript state");
        assert!(transcript.contains("pub struct TranscriptViewport"));
        assert!(transcript.contains("pub enum TranscriptAction"));
        assert!(transcript.contains("pub struct TranscriptEffect"));
        assert!(transcript.contains("pub fn reduce"));
        assert!(transcript.contains("pub fn history_effect"));
        assert!(transcript.contains("pub fn visible_range"));
        assert!(transcript.contains("pub struct TranscriptView"));
        assert!(transcript.contains("pub fn project_view"));
        assert!(transcript.contains("pub fn scroll_to"));
        assert!(transcript.contains("pub fn follow_tail"));
        let app_transcript =
            fs::read_to_string(workspace.join("apps/agena/src/app/app_types/transcript.rs"))
                .expect("read app transcript types");
        assert!(!app_transcript.contains("struct TranscriptViewport"));
        assert!(app_transcript.contains("TranscriptAction,"));
        assert!(app_transcript.contains("TranscriptViewport,"));
        let app_state =
            fs::read_to_string(workspace.join("apps/agena/src/app/transcript_state.rs"))
                .expect("read app transcript state");
        assert!(app_state.contains("TranscriptAction::Reset"));
        assert!(app_state.contains("TranscriptAction::FollowTail"));
        assert!(app_state.contains("TranscriptAction::ScrollTo"));
        assert!(app_state.contains("agena_tui::transcript::project_view"));
        assert!(app_state.contains("history_effect"));
        let keymap = fs::read_to_string(workspace.join("crates/agena-tui/src/keymap/mod.rs"))
            .expect("read TUI keymap");
        assert!(keymap.contains("pub enum KeyAction"));
        assert!(keymap.contains("pub enum KeyContext"));
        let transaction =
            fs::read_to_string(workspace.join("crates/agena-tui/src/terminal_transaction.rs"))
                .expect("read TUI terminal transaction state");
        assert!(transaction.contains("pub enum ProtocolTransactionState"));
        let lifecycle =
            fs::read_to_string(workspace.join("crates/agena-tui/src/terminal_lifecycle.rs"))
                .expect("read TUI terminal lifecycle");
        assert!(lifecycle.contains("pub struct TerminalLifecycle"));
    }

    #[test]
    fn tui_characterization_matrix_covers_lifecycle_transcript_and_rendering() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let lifecycle =
            fs::read_to_string(workspace.join("crates/agena-tui/src/terminal_lifecycle.rs"))
                .expect("read TUI lifecycle characterization tests");
        for test in [
            "every_startup_failure_rolls_back_all_completed_terminal_modes",
            "acknowledging_the_panic_hook_restore_clears_non_idempotent_state",
        ] {
            assert!(lifecycle.contains(test), "missing lifecycle test: {test}");
        }

        let app_tests = fs::read_to_string(workspace.join("apps/agena/src/app/app_tests.rs"))
            .expect("read transcript navigation characterization tests");
        assert!(
            app_tests.contains("transcript_search_starts_after_or_before_the_cursor_and_wraps")
        );

        let mouse = fs::read_to_string(workspace.join("apps/agena/src/app/app_mouse.rs"))
            .expect("read transcript mouse characterization tests");
        for test in [
            "mouse_selection_copies_forward_and_backward_cell_ranges",
            "mouse_selection_preserves_line_breaks_and_partial_endpoints",
            "mouse_selection_never_splits_wide_or_combining_graphemes",
        ] {
            assert!(mouse.contains(test), "missing mouse/copy test: {test}");
        }

        let rendering = fs::read_to_string(workspace.join("apps/agena/src/app/transcript_view.rs"))
            .expect("read transcript rendering characterization tests");
        assert!(
            rendering
                .contains("tool_image_attachments_render_once_through_the_rich_content_pipeline")
        );

        let formulas = fs::read_to_string(
            workspace.join("apps/agena/src/app/transcript_view/transcript_ast.rs"),
        )
        .expect("read formula characterization tests");
        assert!(
            formulas.contains(
                "native_inline_formulas_do_not_duplicate_list_markers_across_graphic_rows"
            )
        );

        let layout =
            fs::read_to_string(workspace.join("crates/agena-tui-components/src/workbench.rs"))
                .expect("read responsive layout characterization tests");
        assert!(layout.contains("fixed_workbench_stacks_on_narrow_terminals_and_splits_when_wide"));
    }

    #[test]
    fn terminal_app_replacement_has_no_legacy_app_directory_or_binary() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(!workspace.join("apps/agena-cli").exists());
        assert!(!workspace.join("tools/agena-cli-test-tools").exists());
        assert!(workspace.join("tools/agena-e2e").exists());
        let app_manifest = fs::read_to_string(workspace.join("apps/agena/Cargo.toml"))
            .expect("read final app manifest");
        assert!(app_manifest.contains("name = \"agena\""));
        assert!(app_manifest.contains("path = \"src/main.rs\""));
        let tui_manifest = fs::read_to_string(workspace.join("crates/agena-tui/Cargo.toml"))
            .expect("read TUI manifest");
        assert!(!tui_manifest.contains("[[bin]]"));
    }

    #[test]
    fn legacy_core_root_has_no_compatibility_reexports() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let root = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
            .expect("read Runtime root module");
        assert!(!workspace.join("crates/agena/Cargo.toml").exists());
        assert!(!root.contains("pub use error::AppError;"));
        assert!(!root.contains("pub use agena_plugin_host as plugin;"));
        let catalog_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog/mod.rs"))
                .expect("read Runtime model catalog adapter");
        assert!(!catalog_module.contains("CatalogModelRecord"));
        let provider = fs::read_to_string(workspace.join("crates/agena-provider/src/lib.rs"))
            .expect("read provider catalog contract");
        assert!(provider.contains("pub struct CatalogModelRecord"));
        assert!(provider.contains("mod catalog_definition;"));
        assert!(provider.contains("CatalogModelDefinition"));
        let ranking =
            fs::read_to_string(workspace.join("crates/agena-provider/src/catalog_definition.rs"))
                .expect("read provider catalog definition and ranking sidecar");
        assert!(ranking.contains("struct CatalogDefinitionSourcePriority"));
        assert!(ranking.contains("never serialized"));
        assert!(
            !workspace
                .join("crates/agena-runtime/src/model_catalog/merge.rs")
                .exists(),
            "core must not retain model-catalog merge or configuration-priority helpers"
        );
        let projection =
            fs::read_to_string(workspace.join("crates/agena-provider/src/catalog_projection.rs"))
                .expect("read provider catalog projection");
        assert!(projection.contains("pub fn catalog_definition_from_model"));
        let merge_primitives =
            fs::read_to_string(workspace.join("crates/agena-provider/src/catalog_merge.rs"))
                .expect("read provider catalog merge primitives");
        assert!(merge_primitives.contains("pub fn merge_model_pricing"));
        assert!(merge_primitives.contains("pub fn merge_catalog_definition"));
        assert!(merge_primitives.contains("pub fn merge_live_provider_catalog_document"));
        let public_merge =
            fs::read_to_string(workspace.join("crates/agena-provider/src/catalog_public_merge.rs"))
                .expect("read provider public-source catalog merge");
        assert!(public_merge.contains("pub fn merge_public_source_catalog_document"));
        let provider_thinking_modes = fs::read_to_string(
            workspace.join("crates/agena-provider/src/catalog_thinking_modes.rs"),
        )
        .expect("read provider catalog thinking-mode enrichment");
        assert!(provider_thinking_modes.contains("pub fn enrich_catalog_document_thinking_modes"));
        let collector =
            fs::read_to_string(workspace.join("crates/agena-provider/src/catalog_collector.rs"))
                .expect("read provider catalog collector");
        assert!(collector.contains("pub async fn collect_live_provider_models"));
        assert!(provider.contains("pub const fn as_persisted"));
        assert!(provider.contains("pub fn from_persisted"));
        assert!(
            !workspace
                .join("crates/agena-runtime/src/model_catalog/service.rs")
                .exists(),
            "core must not retain the catalog service"
        );
        let runtime_catalog_service =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog_service.rs"))
                .expect("read runtime catalog service");
        assert!(runtime_catalog_service.contains("pub struct ModelCatalogService"));
        assert!(runtime_catalog_service.contains("compose_model_catalog_document"));
        assert!(runtime_catalog_service.contains("SnapshotStore<ModelCatalogSnapshot>"));
        assert!(runtime_catalog_service.contains("model_catalog_snapshot_from_cache_record"));
        assert!(runtime_catalog_service.contains("model_catalog_cache_record_from_document"));
        assert!(runtime_catalog_service.contains("build_live_provider_catalog_document"));
        let core_catalog_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog/mod.rs"))
                .expect("read core catalog module");
        assert!(
            !core_catalog_module.contains("pub use agena_runtime::ModelCatalogService"),
            "core must not re-export the runtime-owned catalog service"
        );
        assert!(
            !core_catalog_module.contains("pub use agena_provider::{"),
            "core must not publicly re-export provider-owned catalog values"
        );
        assert!(
            !runtime_catalog_service.contains("async fn collect_live_provider_models"),
            "runtime service must consume, not duplicate, the provider collector"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/model_catalog/curate.rs")
                .exists(),
            "core must not retain pure catalog curation"
        );
        let runtime_curation = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/model_catalog_curation.rs"),
        )
        .expect("read runtime catalog curation");
        let provider_catalog_model_id =
            fs::read_to_string(workspace.join("crates/agena-provider/src/catalog_model_id.rs"))
                .expect("read provider catalog model-ID values");
        assert!(runtime_curation.contains("pub fn curate_catalog_document"));
        assert!(runtime_curation.contains("pub fn curate_live_catalog_document"));
        assert!(
            !runtime_curation.contains("pub fn normalized_catalog_model_id")
                && !runtime_curation.contains("pub fn catalog_model_id_for_raw")
                && provider_catalog_model_id.contains("pub fn normalized_catalog_model_id")
                && provider_catalog_model_id.contains("pub fn catalog_model_id_for_raw"),
            "provider must own canonical catalog model-ID values while Runtime retains curation"
        );
        assert!(!runtime_curation.contains("crate::AppError"));
        let runtime_cache =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog_cache.rs"))
                .expect("read runtime model catalog cache codec");
        assert!(runtime_cache.contains("pub fn model_catalog_snapshot_from_cache_record"));
        assert!(runtime_cache.contains("pub fn model_catalog_cache_record_from_document"));
        let runtime_live_catalog =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog_live.rs"))
                .expect("read runtime live provider catalog composition");
        assert!(runtime_live_catalog.contains("pub async fn build_live_provider_catalog_document"));
        assert!(runtime_live_catalog.contains("collect_live_provider_models"));
        let runtime_catalog_composition = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/model_catalog_composition.rs"),
        )
        .expect("read runtime catalog result composition");
        assert!(runtime_catalog_composition.contains("pub fn compose_model_catalog_document"));
        assert!(runtime_catalog_composition.contains("merge_live_provider_catalog_document"));
        assert!(runtime_catalog_composition.contains("curate_live_catalog_document"));
        assert!(!runtime_catalog_service.contains("curate_live_catalog_document(merged)"));
        let catalog_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog/mod.rs"))
                .expect("read core catalog module");
        assert!(!catalog_module.contains("fn format_catalog_source"));
        assert!(!catalog_module.contains("fn parse_catalog_source"));
        assert!(!catalog_module.contains("fn catalog_model_id_for_raw"));
        assert!(
            !workspace
                .join("crates/agena-runtime/src/model_catalog/sources.rs")
                .exists(),
            "core must not retain a model-catalog public-source wrapper"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/model_catalog/sources")
                .exists(),
            "core must not retain model-catalog parser/enrichment source modules"
        );
        let runtime_sources =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog_source.rs"))
                .expect("read runtime catalog source values");
        assert!(runtime_sources.contains("pub enum ModelCatalogRemoteSourceKind"));
        assert!(runtime_sources.contains("pub struct ModelCatalogRemoteSourceGrade"));
        assert!(runtime_sources.contains("pub struct ModelCatalogRemoteSource"));
        let runtime_http = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/model_catalog_http/mod.rs"),
        )
        .expect("read runtime catalog HTTP adapter");
        assert!(runtime_http.contains("pub fn build_model_catalog_public_source"));
        assert!(runtime_http.contains("pub fn build_default_public_model_catalog_source"));
        assert!(runtime_http.contains("HttpModelCatalogDocumentFetcher"));
        assert!(runtime_sources.contains("pub fn default_model_catalog_source_grade"));
        let storage = fs::read_to_string(workspace.join("crates/agena-storage/src/lib.rs"))
            .expect("read storage transaction effects contract");
        assert!(storage.contains("pub struct TransactionEffects"));
        assert!(
            !workspace
                .join("crates/agena-runtime/src/database_transaction.rs")
                .exists(),
            "runtime must not retain a concrete SeaORM transaction runner"
        );
        let sqlite_transaction =
            fs::read_to_string(workspace.join("crates/agena-storage-sqlite/src/transaction.rs"))
                .expect("read SQLite transaction adapter");
        assert!(sqlite_transaction.contains("use agena_storage::TransactionEffects;"));
        assert!(sqlite_transaction.contains("pub async fn run_transaction_effects"));
        assert!(!workspace.join("crates/agena-runtime/src/db/tx.rs").exists());
        assert!(
            !workspace
                .join("crates/agena-runtime/src/model_catalog/types.rs")
                .exists(),
            "core must not retain a duplicate catalog-definition value"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/model_catalog/ranking.rs")
                .exists(),
            "core must not retain a duplicate catalog-ranking sidecar"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/session/execution_registry.rs")
                .exists(),
            "core must not retain the generic execution registry"
        );
        let runtime_execution_registry =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/execution_registry.rs"))
                .expect("read runtime execution registry");
        assert!(runtime_execution_registry.contains("pub struct ExecutionRegistry<T>"));
        assert!(runtime_execution_registry.contains("pub struct ExecutionControl<T>"));
        let session_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/mod.rs"))
                .expect("read core session module");
        assert!(session_module.contains(
            "type ExecutionRegistry = agena_runtime::ExecutionRegistry<crate::message::PartContent>"
        ));
        let runtime_prompt_budget =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/prompt_budget.rs"))
                .expect("read runtime prompt budget");
        assert!(runtime_prompt_budget.contains("pub fn prompt_token_budget"));
        assert!(runtime_prompt_budget.contains("pub const APPROX_CHARS_PER_TOKEN"));
        assert!(runtime_prompt_budget.contains("pub const MIN_PROMPT_BUDGET_TOKENS"));
        let core_prompt_window =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/prompt_window.rs"))
                .expect("read core prompt window");
        assert!(!core_prompt_window.contains("fn prompt_token_budget"));
        assert!(core_prompt_window.contains("agena_runtime::prompt_token_budget"));
        assert!(core_prompt_window.contains("agena_runtime::APPROX_CHARS_PER_TOKEN"));
        assert!(core_prompt_window.contains("agena_runtime::MIN_PROMPT_BUDGET_TOKENS"));
        let runtime_cache_policy =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session_cache_policy.rs"))
                .expect("read runtime session cache policy");
        assert!(runtime_cache_policy.contains("pub struct SessionCachePolicy"));
        let core_session_cache =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/cache.rs"))
                .expect("read core session cache");
        assert!(!core_session_cache.contains("struct SessionCachePolicy"));
        assert!(core_session_cache.contains("use agena_runtime::SessionCachePolicy"));
        assert!(
            !workspace
                .join("crates/agena-runtime/src/session/context_governor.rs")
                .exists(),
            "core must not retain the context-governor policy"
        );
        let runtime_context_governor =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/context_governor.rs"))
                .expect("read runtime context governor");
        assert!(runtime_context_governor.contains("pub struct ContextGovernor"));
        let core_processor_run =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/processor/run.rs"))
                .expect("read core processor run adapter");
        assert!(core_processor_run.contains("approximate_prompt_payload_chars(messages)"));
        let runtime_context_budget =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/context_budget.rs"))
                .expect("read runtime context budget");
        assert!(runtime_context_budget.contains("pub fn estimate_auto_compaction_limit_tokens"));
        let session_root =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/mod.rs"))
                .expect("read core session root");
        assert!(!session_root.contains("fn estimate_auto_compaction_limit_tokens"));
        let runtime_session_cache =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session_cache.rs"))
                .expect("read runtime generic session cache");
        assert!(runtime_session_cache.contains("pub trait CacheEntry"));
        assert!(runtime_session_cache.contains("pub struct SessionCache<T: CacheEntry>"));
        let core_session_cache =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/cache.rs"))
                .expect("read core session cache adapter");
        assert!(core_session_cache.contains("impl agena_runtime::CacheEntry for Session"));
        assert!(!core_session_cache.contains("struct CachedSessionRecord"));
        let runtime_session_requests =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session_requests.rs"))
                .expect("read runtime session request values");
        for value in [
            "pub struct SessionCreateRequest",
            "pub struct SessionForkRequest",
            "pub struct SessionRewindRequest",
            "pub struct SessionRunOptions",
            "pub struct SessionExecutionRequest",
            "pub struct SessionExecutionReplyRequest<T>",
            "pub struct SessionUserMessageRequest<T>",
            "pub struct SessionPermissionReplyRequest",
            "pub struct SessionAgentSwitchOutcome",
            "pub struct SessionAgentRestoreOutcome",
        ] {
            assert!(runtime_session_requests.contains(value));
        }
        let core_session_manager =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/manager/mod.rs"))
                .expect("read core session manager adapter");
        assert!(!core_session_manager.contains("pub struct SessionCreateRequest"));
        assert!(!core_session_manager.contains("pub struct SessionForkRequest"));
        assert!(!core_session_manager.contains("pub struct SessionRewindRequest"));
        assert!(!core_session_manager.contains("pub struct SessionRunOptions"));
        assert!(!core_session_manager.contains("pub struct SessionExecutionRequest"));
        assert!(!core_session_manager.contains("pub struct SessionExecutionReplyRequest<T>"));
        assert!(!core_session_manager.contains("pub struct SessionUserMessageRequest"));
        assert!(!core_session_manager.contains("pub struct SessionPermissionReplyRequest"));
        assert!(
            core_session_manager.contains("agena_runtime::SessionUserMessageRequest<PartContent>")
        );
        assert!(!core_session_manager.contains("pub use agena_runtime::{"));
        let core_session_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/mod.rs"))
                .expect("read core session module facade");
        for legacy_export in [
            "SessionCreateRequest",
            "SessionForkRequest",
            "SessionRewindRequest",
            "SessionRunOptions",
            "SessionExecutionRequest",
            "SessionExecutionReplyRequest",
            "SessionUserMessageRequest",
            "SessionPermissionReplyRequest",
        ] {
            assert!(
                !core_session_module.contains(legacy_export),
                "core session facade must not re-export runtime request value {legacy_export}"
            );
        }
        let domain_session_cost =
            fs::read_to_string(workspace.join("crates/agena-domain/src/session_cost.rs"))
                .expect("read domain session cost values");
        assert!(domain_session_cost.contains("pub struct ModelCostBreakdown"));
        assert!(domain_session_cost.contains("pub struct SessionCostSummary"));
        let domain_usage_query =
            fs::read_to_string(workspace.join("crates/agena-domain/src/usage_query.rs"))
                .expect("read domain usage query value");
        assert!(domain_usage_query.contains("pub struct UsageStatsQuery"));
        assert!(domain_usage_query.contains("pub fn matches("));
        let domain_usage_stats =
            fs::read_to_string(workspace.join("crates/agena-domain/src/usage_stats.rs"))
                .expect("read domain usage result values");
        for value in [
            "pub struct UsageTotals",
            "pub struct UsageDailyBreakdown",
            "pub struct ProviderUsageBreakdown",
            "pub struct ModelUsageBreakdown",
            "pub struct SessionUsageBreakdown",
            "pub struct UsageStats",
        ] {
            assert!(domain_usage_stats.contains(value));
        }
        let domain_session_summary =
            fs::read_to_string(workspace.join("crates/agena-domain/src/session_summary.rs"))
                .expect("read domain session read-model values");
        assert!(domain_session_summary.contains("pub struct SessionListRequest"));
        assert!(domain_session_summary.contains("pub struct SessionSummary"));
        let core_session_model =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/model.rs"))
                .expect("read core session model");
        let core_session_root =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/mod.rs"))
                .expect("read core session root");
        for value in ["SessionListRequest", "SessionSummary"] {
            assert!(!core_session_model.contains(&format!("pub struct {value}")));
            assert!(!core_session_root.contains(value));
        }
        let core_session_cost =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/cost.rs"))
                .expect("read core session cost aggregation adapter");
        assert!(!core_session_cost.contains("pub struct ModelCostBreakdown"));
        assert!(!core_session_cost.contains("pub struct SessionCostSummary"));
        assert!(!core_session_cost.contains("pub struct UsageStatsQuery"));
        assert!(!core_session_cost.contains("pub struct UsageTotals"));
        assert!(!core_session_cost.contains("pub struct UsageStats"));
        assert!(core_session_cost.contains("fn fold_model_cost("));
        let runtime_usage_stats =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/usage_stats.rs"))
                .expect("read Runtime usage-stat aggregation");
        assert!(runtime_usage_stats.contains("pub fn summarize_usage_records"));
        assert!(runtime_usage_stats.contains("agena_provider::completion_usage_cost_contribution"));
        assert!(core_session_manager.contains("fn completion_request("));
        assert!(core_session_manager.contains("agena_runtime::build_completion_request"));
        let runtime_prompt_merge =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/prompt_merge.rs"))
                .expect("read runtime system-prompt merge policy");
        assert!(runtime_prompt_merge.contains("pub fn merge_system_prompts"));
        assert!(!core_session_manager.contains("pub(super) fn merge_system_prompts"));
        assert!(core_session_manager.contains("agena_runtime::merge_system_prompts"));
        let runtime_snapshot_registry =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/snapshot_registry.rs"))
                .expect("read runtime snapshot registry");
        assert!(runtime_snapshot_registry.contains("pub struct SnapshotSession"));
        assert!(runtime_snapshot_registry.contains("pub type SnapshotRegistry"));
        assert!(runtime_snapshot_registry.contains("pub fn list_active_snapshots"));
        let runtime_snapshot_capabilities =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/snapshot_capabilities.rs"))
                .expect("read runtime snapshot capability probe");
        assert!(
            runtime_snapshot_capabilities.contains("pub(crate) fn snapshot_backend_capabilities")
        );
        let runtime_control = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime_control_service.rs"),
        )
        .expect("read runtime snapshot capability control port");
        let runtime_root = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
            .expect("read Runtime root exports");
        let application_handle =
            fs::read_to_string(workspace.join("crates/agena-application/src/application.rs"))
                .expect("read application snapshot capability adapter");
        let application_git =
            fs::read_to_string(workspace.join("crates/agena-application/src/service/git.rs"))
                .expect("read application snapshot status projection");
        let runtime_builder =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/builder.rs"))
                .expect("read runtime snapshot-capability control adapter");
        assert!(runtime_control.contains(
            "fn snapshot_backend_capabilities(&self, workspace: &Path) -> SnapshotBackendCapabilities;"
        ));
        assert!(
            runtime_root
                .contains("pub(crate) use snapshot_capabilities::snapshot_backend_capabilities;")
        );
        assert!(
            !runtime_root.contains("pub use snapshot_capabilities::snapshot_backend_capabilities;")
        );
        assert!(
            application_handle.contains("RuntimeControlService::snapshot_backend_capabilities(")
        );
        assert!(runtime_builder.contains("fn snapshot_backend_capabilities("));
        assert!(!application_git.contains("agena_runtime::snapshot_backend_capabilities"));
        let runtime_snapshot_managed =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/snapshot_managed.rs"))
                .expect("read runtime managed snapshot service");
        assert!(runtime_snapshot_managed.contains("pub struct ManagedSnapshot"));
        assert!(runtime_snapshot_managed.contains("pub fn list_managed_snapshots"));
        assert!(runtime_snapshot_managed.contains("pub fn prune_stale_managed_snapshots"));
        let runtime_snapshot_operations =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/snapshot_operations.rs"))
                .expect("read runtime snapshot operations");
        assert!(runtime_snapshot_operations.contains("pub fn create_managed_snapshot"));
        assert!(runtime_snapshot_operations.contains("pub fn attach_existing_snapshot"));
        assert!(runtime_snapshot_operations.contains("pub fn remove_managed_snapshot"));
        let core_snapshot =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/tool/snapshot.rs"))
                .expect("read core snapshot tool adapter");
        assert!(!core_snapshot.contains("pub struct SnapshotSession"));
        assert!(!core_snapshot.contains("pub struct ActiveSnapshot"));
        assert!(!core_snapshot.contains("pub use agena_runtime"));
        assert!(core_snapshot.contains("SnapshotRegistry"));
        assert!(core_snapshot.contains("SnapshotSession"));
        assert!(!core_snapshot.contains("fn probe_git_backend"));
        assert!(!core_snapshot.contains("fn probe_command_presence"));
        assert!(!core_snapshot.contains("pub struct ManagedSnapshot"));
        assert!(!core_snapshot.contains("fn list_managed("));
        assert!(!core_snapshot.contains("fn prune_stale("));
        assert!(!core_snapshot.contains("Command::new"));
        assert!(!core_snapshot.contains("SnapshotBackend::"));
        assert!(
            !workspace
                .join("crates/agena-runtime/src/tool/monitor.rs")
                .exists(),
            "core must not retain the concrete background-process registry"
        );
        let runtime_monitor =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/monitor.rs"))
                .expect("read runtime background-process registry");
        assert!(runtime_monitor.contains("pub struct MonitorRegistry"));
        assert!(runtime_monitor.contains("pub trait MonitorService"));
        let core_tool_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/tool/mod.rs"))
                .expect("read core tool adapter module");
        assert!(core_tool_module.contains("pub(crate) use agena_runtime::{"));
        assert!(core_tool_module.contains("MonitorService"));
        assert!(!core_tool_module.contains("struct MonitorRegistry"));
        let core_executor_execution = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/tool/executor/executor_execution.rs"),
        )
        .expect("read core tool execution adapter");
        assert!(core_executor_execution.contains("pub fn execute_invocation_summary"));
        assert!(core_executor_execution.contains("agena_tool::ToolExecutionSummary"));
        let runtime_tool_output =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/tool_output.rs"))
                .expect("read runtime tool output policy");
        assert!(runtime_tool_output.contains("pub fn truncate_tool_output_text"));
        let core_truncation =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/tool/truncation.rs"))
                .expect("read core tool truncation adapter");
        assert!(!core_truncation.contains("fn truncate_text("));
        assert!(core_truncation.contains("agena_runtime::truncate_tool_output_text"));
        assert!(
            workspace
                .join("crates/agena-storage-sqlite/src/transaction.rs")
                .is_file(),
            "SQLite storage must own the concrete transaction runner"
        );
        assert!(!workspace.join("crates/agena-runtime/src/db/tx.rs").exists());
        for relative in [
            "apps/agena/src",
            "crates/agena-application/src",
            "crates/agena-api-server/src",
        ] {
            let source = collect_rust_sources(&workspace.join(relative))
                .expect("read AppError consumers")
                .join("\n");
            assert!(
                !source.contains("agena::AppError"),
                "external consumer must use `agena::error::AppError`: {relative}"
            );
        }
        let terminal_main = fs::read_to_string(workspace.join("apps/agena/src/main.rs"))
            .expect("read terminal app-server bootstrap");
        let terminal_lib = fs::read_to_string(workspace.join("apps/agena/src/lib.rs"))
            .expect("read terminal library boundary");
        assert!(
            terminal_lib.contains("pub enum AgenaAppError")
                && !terminal_main.contains("agena::error::AppError")
                && !terminal_lib.contains("agena::error::AppError"),
            "terminal package must own its process/presentation error boundary"
        );
        let terminal_production = collect_rust_sources(&workspace.join("apps/agena/src"))
            .expect("read terminal production boundary")
            .into_iter()
            .filter(|source| !source.contains("#[cfg(test)]"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "agena::config::",
            "agena::error::",
            "agena::event::",
            "agena::message::",
            "agena::provider::",
            "agena::session::",
        ] {
            assert!(
                !terminal_production.contains(forbidden),
                "terminal production source must not reopen Core seam `{forbidden}`"
            );
        }
        assert!(
            terminal_main.contains("bootstrap_application_services(")
                && terminal_main.contains("request.config_override_expressions.clone()")
                && terminal_main.contains("RuntimeBootstrapResult")
                && !terminal_main.contains("AgenaRuntimeConfig {"),
            "terminal app-server must retain the Runtime bootstrap result rather than a concrete Core runtime"
        );
        assert!(
            terminal_main.contains("services: RuntimeApplicationServices")
                && terminal_main.contains("app_session_queries(&self.services)")
                && terminal_main.contains("app_session_commands(&self.services)")
                && terminal_main.contains("app_session_control(&self.services)")
                && !terminal_main.contains("app_session_manager(")
                && !terminal_main.contains("current_snapshot()"),
            "terminal app-server must consume Runtime application/session ports rather than Core runtime snapshots"
        );
        let terminal_transcript =
            fs::read_to_string(workspace.join("apps/agena/src/app/transcript_state.rs"))
                .expect("read terminal transcript live-event adapter");
        let terminal_transcript_tests =
            fs::read_to_string(workspace.join("apps/agena/src/app/app_tests.rs"))
                .expect("read terminal transcript projection tests");
        let application_message_projection =
            fs::read_to_string(workspace.join("crates/agena-application/src/service/messages.rs"))
                .expect("read application message-part projection");
        assert!(
            terminal_transcript.contains("RuntimeMessagePartCheckpoint")
                && terminal_transcript.contains("message_part_resource_from_runtime(")
                && terminal_transcript.contains("&update.part")
                && !terminal_transcript.contains("message_part_resource_from_domain")
                && !terminal_transcript.contains("apply_live_event"),
            "terminal production live transcript updates must consume the typed Runtime checkpoint before API projection"
        );
        assert!(
            terminal_transcript_tests.contains("RuntimePresentationEventKind")
                && terminal_transcript_tests.contains("apply_presentation_event")
                && !terminal_transcript_tests.contains("event::{DomainEvent"),
            "terminal transcript tests must exercise the same Runtime presentation projection as production"
        );
        assert!(
            application_message_projection.contains("pub fn message_part_resource_from_runtime(")
                && !application_message_projection.contains("agena::message::MessagePart"),
            "Application message-part projection must accept Runtime values without importing Core aggregates"
        );
        let terminal_tui = fs::read_to_string(workspace.join("apps/agena/src/lib.rs"))
            .expect("read terminal TUI bootstrap");
        assert!(
            terminal_tui.contains("bootstrap_application_services(")
                && terminal_tui.contains("runtime.application_services()")
                && terminal_tui.contains("runtime.shutdown();")
                && terminal_tui.contains("Backend::new(runtime.application_services()")
                && terminal_tui.contains("let tui_preferences = backend.ui_configuration();")
                && terminal_tui.contains("tui_config_from_preferences(&tui_preferences)")
                && !terminal_tui.contains("runtime_configuration()")
                && terminal_tui.contains(
                    "config_override_expressions: args.config_override_expressions.clone()"
                )
                && !terminal_tui.contains("AgenaRuntimeConfig {"),
            "terminal embedded TUI must use the Runtime bootstrap request and Application UI projection rather than Core or Runtime config construction"
        );
        let studio_app = fs::read_to_string(workspace.join("apps/agena-studio-server/src/app.rs"))
            .expect("read studio runtime consumer");
        let application_runtime =
            fs::read_to_string(workspace.join("crates/agena-application/src/application.rs"))
                .expect("read Application runtime diagnostic projection");
        assert!(
            studio_app.contains("application: Application")
                && studio_app.contains("state.application.runtime_diagnostics().await")
                && studio_app.contains("Application::from_composed_runtime_services")
                && application_runtime.contains("pub async fn runtime_diagnostics(&self)")
                && !studio_app.contains("RuntimeApplicationServices")
                && !studio_app.contains("state.runtime"),
            "Studio state must consume Application diagnostics/workspace use cases rather than retain a Runtime service bundle"
        );
        let cli_plugin_commands =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_run.rs"))
                .expect("read CLI plugin commands");
        assert!(
            cli_plugin_commands.contains("runtime.application_services().plugins")
                && !cli_plugin_commands.contains("runtime.current_snapshot().plugin_manager()"),
            "CLI plugin commands must use PluginRuntimeService rather than traverse a Core snapshot"
        );
        let cli_runtime =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_runtime.rs"))
                .expect("read CLI runtime consumers");
        assert!(
            cli_runtime.contains("self.with_application(|application| async move")
                && cli_runtime.contains(".agent_statuses()")
                && cli_runtime.contains(".default_agent_name()")
                && !cli_runtime.contains(".status.runtime_status().await")
                && !cli_runtime.contains("snapshot.agents().list_descriptors()"),
            "CLI agent listing must use Application agent projections rather than Runtime status or a Core agent registry"
        );
        assert!(
            cli_runtime.contains("tools: services.tools")
                && !cli_runtime.contains("runtime.current_snapshot()")
                && !cli_runtime.contains("runtime.session_manager()")
                && !cli_runtime.contains("ToolExecutor::new("),
            "CLI MCP bootstrap must consume Runtime tool services rather than compose a Core executor"
        );
        let cli_runtime_helpers =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_runtime_helpers.rs"))
                .expect("read CLI run-option helpers");
        assert!(
            cli_runtime_helpers.contains("providers: &dyn ProviderCatalog")
                && !cli_runtime_helpers.contains("AgenaRuntime")
                && !cli_runtime_helpers.contains("current_snapshot()"),
            "CLI run-option helpers must resolve models through ProviderCatalog rather than a Core runtime snapshot"
        );
        assert!(
            cli_runtime.contains("self.with_session_runtime_services(|services| async move")
                && cli_runtime.contains("let providers = services.provider_catalog;")
                && cli_runtime.contains(".list_providers()")
                && cli_runtime.contains(".model_execution_options(&model_ref)")
                && cli_runtime.contains("bootstrap_application_services(")
                && !cli_runtime.contains("build_provider_registry_from_configs"),
            "CLI provider commands must use ProviderCatalog through the Runtime bootstrap result rather than construct a Core provider registry"
        );
        let cli_render =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_render.rs"))
                .expect("read CLI snapshot consumers");
        assert!(
            cli_render.contains("application_from_runtime(&runtime)")
                && cli_render.contains("application.snapshot_status()")
                && cli_render.contains(".git_status()")
                && !cli_render.contains("SessionExecutionControl::snapshot_registry")
                && !cli_render.contains("list_active_snapshots")
                && !cli_render.contains("list_managed_snapshots")
                && !cli_render.contains("manager.tool_executor()")
                && !cli_render.contains("session_manager()"),
            "CLI snapshot and git rendering must consume application projections rather than inspect Runtime registry state"
        );
        for relative in [
            "crates/agena-application/src/error.rs",
            "crates/agena-api-server/src/error.rs",
        ] {
            let source = fs::read_to_string(workspace.join(relative))
                .expect("read transport/application error boundary");
            assert!(
                !source.contains("AppError"),
                "application and API error boundaries must collapse core errors before transport: {relative}"
            );
        }
        for forbidden in [
            "pub use agena_domain",
            "pub use agena_provider",
            "pub use agena_tool",
        ] {
            assert!(
                !root.contains(forbidden),
                "legacy core root must not restore {forbidden}"
            );
        }
    }

    #[test]
    fn application_uses_plugin_host_directly() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = fs::read_to_string(workspace.join("crates/agena-application/Cargo.toml"))
            .expect("read application manifest");
        assert!(manifest.contains("agena-plugin-host = { workspace = true }"));
        let source = collect_rust_sources(&workspace.join("crates/agena-application/src"))
            .expect("read application plugin consumers")
            .join("\n");
        assert!(source.contains("agena_plugin_host::"));
        assert!(!source.contains("agena::plugin"));
    }

    #[test]
    fn filesystem_memory_repository_is_owned_by_storage() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let storage =
            fs::read_to_string(workspace.join("crates/agena-storage/src/memory_store.rs"))
                .expect("read storage memory adapter");
        assert!(storage.contains("pub struct MemoryStore"));
        assert!(storage.contains("impl MemoryRepository for MemoryStore"));
        assert!(
            !workspace
                .join("crates/agena-runtime/src/memory/store.rs")
                .exists(),
            "core must not retain the filesystem memory repository"
        );
        let runtime_memory =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/memory/plugin.rs"))
                .expect("read Runtime memory plugin");
        assert!(runtime_memory.contains("MemoryStore"));
        assert!(
            !workspace.join("crates/agena/src/memory").exists(),
            "deleted Core monolith must not retain a memory implementation"
        );
    }

    #[test]
    fn api_server_uses_plugin_host_directly() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = fs::read_to_string(workspace.join("crates/agena-api-server/Cargo.toml"))
            .expect("read API server manifest");
        assert!(manifest.contains("agena-plugin-host = { workspace = true }"));
        let source = collect_rust_sources(&workspace.join("crates/agena-api-server/src"))
            .expect("read API server plugin consumers")
            .join("\n");
        assert!(source.contains("agena_plugin_host::"));
        assert!(!source.contains("agena::plugin"));
    }

    #[test]
    fn cli_uses_plugin_host_directly() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = fs::read_to_string(workspace.join("crates/agena-cli/Cargo.toml"))
            .expect("read CLI manifest");
        assert!(manifest.contains("agena-plugin-host = { workspace = true }"));
        assert!(manifest.contains("plugin-signing = [\"agena-plugin-host/signing\"]"));
        let source = collect_rust_sources(&workspace.join("crates/agena-cli/src"))
            .expect("read CLI plugin consumers")
            .join("\n");
        assert!(source.contains("agena_plugin_host::"));
        assert!(!source.contains("agena::plugin"));
    }

    #[test]
    fn cli_permission_composition_consumes_runtime_owned_repository_ports() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = fs::read_to_string(workspace.join("crates/agena-cli/Cargo.toml"))
            .expect("read CLI manifest");
        assert!(
            !manifest.contains("agena-storage-sqlite = { workspace = true }")
                && !manifest.contains("sea-orm.workspace = true"),
            "CLI must not compose SQLite adapters outside Runtime"
        );
        let permissions =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_permissions.rs"))
                .expect("read CLI permission commands");
        assert!(
            permissions.contains("Application::from_composed_runtime_services")
                && !permissions.contains("ApplicationRepositories")
                && !permissions.contains("SeaWorkspaceRepository")
                && permissions.contains("fn application_from_runtime("),
            "CLI composition must consume Runtime-owned repository ports"
        );
        assert!(
            !permissions.contains("workspace_crud"),
            "CLI permission writes must not call core workspace CRUD"
        );
    }

    #[test]
    fn production_application_consumers_do_not_compose_sqlite_repositories() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for manifest in [
            "apps/agena/Cargo.toml",
            "crates/agena-cli/Cargo.toml",
            "apps/agena-studio-server/Cargo.toml",
            "crates/agena-api-server/Cargo.toml",
        ] {
            let source = fs::read_to_string(workspace.join(manifest))
                .expect("read production application consumer manifest");
            assert!(
                !source.contains("agena-storage-sqlite") && !source.contains("sea-orm"),
                "production application consumer must not retain concrete adapter dependency: {manifest}"
            );
        }
        for source_path in [
            "apps/agena/src/backend/backend_workspace.rs",
            "crates/agena-cli/src/cli/cli_permissions.rs",
            "apps/agena-studio-server/src/app.rs",
        ] {
            let source = fs::read_to_string(workspace.join(source_path))
                .expect("read production application consumer composition");
            assert!(
                source.contains("Application::from_composed_runtime_services")
                    && !source.contains("DatabaseConnection")
                    && !source.contains("SeaWorkspaceRepository")
                    && !source.contains("ApplicationRepositories"),
                "production application consumer must consume Runtime repository ports: {source_path}"
            );
        }
        let runtime_services =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/application_services.rs"))
                .expect("read Runtime application service bundle");
        assert!(
            runtime_services.contains("pub struct RuntimeApplicationRepositories")
                && runtime_services
                    .contains("pub repositories: Option<RuntimeApplicationRepositories>")
                && runtime_services
                    .contains("pub(crate) struct RuntimeApplicationServiceCompositionInputs")
                && runtime_services.contains("pub(crate) fn compose_runtime_application_services("),
            "Runtime must expose contract-typed application repositories while keeping concrete service assembly private"
        );
        let terminal_memory =
            fs::read_to_string(workspace.join("apps/agena/src/backend/backend_session.rs"))
                .expect("read terminal memory adapter");
        let application_memory =
            fs::read_to_string(workspace.join("crates/agena-application/src/service/memory.rs"))
                .expect("read application memory service");
        assert!(
            !terminal_memory.contains("MemoryStore")
                && !terminal_memory.contains("memory_store()")
                && terminal_memory.contains(".memory_index_path()")
                && terminal_memory.contains(".memory_entry_path(name)")
                && terminal_memory.contains(".forget_memory(name)")
                && application_memory.contains("pub fn memory_index_path")
                && application_memory.contains("pub fn memory_entry_path")
                && application_memory.contains("pub fn forget_memory"),
            "terminal memory effects must use Application storage contracts rather than rebuilding MemoryStore"
        );
        let cli_memory =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_render.rs"))
                .expect("read CLI memory adapter");
        assert!(
            cli_memory.contains("Application::from_composed_runtime_services")
                && cli_memory.contains("session_runtime_with_workspace(workspace)")
                && !cli_memory.contains("MemoryStore")
                && !cli_memory.contains("MemoryRepository"),
            "CLI memory commands must bootstrap the selected workspace and consume Application memory use cases"
        );
    }

    #[test]
    fn cli_permission_commands_use_application_service_over_sqlite_adapters() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = fs::read_to_string(workspace.join("crates/agena-cli/Cargo.toml"))
            .expect("read CLI manifest");
        assert!(
            !manifest.contains("agena-storage-sqlite = { workspace = true }")
                && !manifest.contains("sea-orm.workspace = true"),
            "CLI must not retain concrete SQLite adapter dependencies"
        );
        let helpers =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_permissions.rs"))
                .expect("read CLI permission composition");
        assert!(
            helpers.contains("PermissionRuleWriteCommand")
                && helpers.contains("Application::from_composed_runtime_services")
                && !helpers.contains("PersistedPermissionRule")
                && !helpers.contains("SeaPermissionRuleRepository")
                && !helpers.contains(".upsert(")
                && !helpers.contains(".replace(")
                && !helpers.contains(".revoke("),
            "CLI permission helpers must compose Application from Runtime services and delegate mutations without a SQLite adapter"
        );
        let render = fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_render.rs"))
            .expect("read CLI permission rendering");
        assert!(
            render.contains("create_permission_rule_command(command)")
                && render.contains("replace_permission_rule_command(args.rule_id, command)")
                && render.contains("revoke_permission_rule_as(")
                && render.contains("list_permission_rules(")
                && !render.contains("SeaPermissionRuleRepository")
                && !render.contains("permission_database("),
            "CLI rendering must use application-service permission operations rather than direct SQLite calls"
        );
        for forbidden in [
            "permission_rule_crud",
            "crud::permission",
            "entities::permission_rule",
        ] {
            assert!(
                !helpers.contains(forbidden) && !render.contains(forbidden),
                "CLI permission path must not use core permission persistence: {forbidden}"
            );
        }
    }

    #[test]
    fn final_app_uses_plugin_host_directly() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = fs::read_to_string(workspace.join("apps/agena/Cargo.toml"))
            .expect("read final app manifest");
        assert!(manifest.contains("agena-plugin-host = { workspace = true }"));
        let source = collect_rust_sources(&workspace.join("apps/agena/src"))
            .expect("read final app plugin consumers")
            .join("\n");
        assert!(source.contains("agena_plugin_host::"));
        assert!(!source.contains("agena::plugin"));
    }

    #[test]
    fn tui_sources_do_not_reintroduce_runtime_or_core_types() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for source in
            collect_rust_sources(&workspace.join("crates/agena-tui/src")).expect("read TUI sources")
        {
            for forbidden in [
                "use agena::",
                "agena::runtime",
                "agena_application",
                "agena_runtime",
                "sea_orm",
                "ProviderRegistry",
                "SessionManager",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "TUI source must not depend on {forbidden}"
                );
            }
        }
    }

    #[test]
    fn ci_workflow_keeps_locked_architecture_and_dependency_gates() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let workflow = fs::read_to_string(workspace.join(".github/workflows/ci.yml"))
            .expect("read CI workflow");
        for required in [
            "cargo fmt --all --check",
            "cargo clippy --workspace --all-targets --locked -- -D warnings",
            "cargo run -p architecture-check --locked",
            "cargo test --workspace --locked",
            "cargo test -p agena-e2e --locked",
            "cargo check -p agena-plugin-host --all-features --locked",
            "cargo check -p agena-runtime --all-features --locked",
            "cargo check -p agena-api-server --all-features --locked",
            "cargo check -p agena-cli --all-features --locked",
            "cargo check -p agena-marketplace-server --features server --locked",
            "cargo install cargo-machete --version 0.9.2 --locked",
            "cargo machete",
            "cargo-deny-action",
        ] {
            assert!(
                workflow.contains(required),
                "CI workflow must retain `{required}`"
            );
        }
    }

    #[test]
    fn developer_entrypoints_do_not_reference_deleted_v1_app_paths() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let timing_probe = workspace.join("scripts/check-build-timings.sh");
        assert!(timing_probe.is_file(), "missing build timing probe");
        let timing_source = fs::read_to_string(timing_probe).expect("read build timing probe");
        assert!(timing_source.contains("ENFORCE_BUILD_TIMING"));
        assert!(timing_source.contains("MEASURE_LEAF_CHANGES"));
        assert!(timing_source.contains("MEASURE_COLD_START"));
        assert!(timing_source.contains("must be 0 or 1"));
        assert!(timing_source.contains("agena-cold-target"));
        assert!(timing_source.contains("CARGO_INCREMENTAL=0"));
        assert!(timing_source.contains("trap cleanup_cold_target EXIT HUP INT TERM"));
        assert!(timing_source.contains("check_retained_target_size"));
        assert!(timing_source.contains("assert_tui_leaf_rebuild_attribution"));
        assert!(timing_source.contains("agena_provider_bedrock_streaming"));
        assert!(timing_source.contains("agena_storage_sqlite"));
        assert!(timing_source.contains("agena_api_server"));
        assert!(timing_source.contains("agena_client"));
        assert!(timing_source.contains("cargo check -p agena-tui --locked"));
        assert!(timing_source.contains("cargo check -p agena-cli --locked"));
        assert!(timing_source.contains("cargo build -p agena --locked"));
        let ci = fs::read_to_string(workspace.join(".github/workflows/ci.yml"))
            .expect("read CI workflow");
        assert!(ci.contains("name: build timing report"));
        assert!(ci.contains(
            "MEASURE_LEAF_CHANGES=1 MEASURE_COLD_START=1 scripts/check-build-timings.sh"
        ));
        assert!(ci.contains("name: build-timing-report"));
        assert!(timing_source.contains("cargo build --locked"));
        let readme = fs::read_to_string(workspace.join("README.md")).expect("read README");
        for required in [
            "cargo check --workspace --all-targets --locked",
            "cargo clippy --workspace --all-targets --locked -- -D warnings",
            "cargo test --workspace --locked",
        ] {
            assert!(
                readme.contains(required),
                "README verification commands must retain `{required}`"
            );
        }
        for relative in [
            "README.md",
            ".github/workflows/ci.yml",
            ".github/workflows/dependency-report.yml",
            "ops/dependencies/check.sh",
            "scripts/provider_gateway_cache_probe.py",
        ] {
            let source = fs::read_to_string(workspace.join(relative))
                .unwrap_or_else(|_| panic!("read developer entrypoint {relative}"));
            for forbidden in [
                "apps/agena-cli",
                "apps/agena-cli/src/bin/agena-tui",
                "agena-app",
                "agena-cli-test-tools",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "developer entrypoint {relative} references deleted path {forbidden}"
                );
            }
        }
    }

    #[test]
    fn persistent_schema_has_explicit_versioned_migration_contract() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let lifecycle = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/schema_lifecycle.rs"),
        )
        .expect("read SQLite schema lifecycle module");
        assert!(lifecycle.contains("pub const CURRENT_SCHEMA_VERSION: i64 = 1"));
        assert!(lifecycle.contains("PRAGMA user_version"));
        assert!(lifecycle.contains("async fn apply_migrations"));
        assert!(lifecycle.contains("newer than supported version"));
        assert!(
            workspace
                .join("crates/agena-storage-sqlite/src/schema.rs")
                .is_file(),
            "SQLite storage must own concrete table and index definitions"
        );
    }

    #[test]
    fn cli_permission_mutations_use_application_service_commands() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = fs::read_to_string(workspace.join("crates/agena-cli/Cargo.toml"))
            .expect("read CLI manifest");
        assert!(
            manifest.contains("agena-application = { workspace = true }"),
            "CLI must retain its direct application-service dependency"
        );
        let permission_helpers =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_permissions.rs"))
                .expect("read CLI permission helpers");
        for required in [
            "PermissionRuleWriteCommand",
            "fn application_from_runtime(",
            "async fn list_permission_rules(",
            "Application::from_composed_runtime_services(runtime.application_services())",
        ] {
            assert!(
                permission_helpers.contains(required),
                "CLI permission composition must retain `{required}`"
            );
        }
        for forbidden in [
            "PersistedPermissionRule",
            ".upsert(",
            ".replace(",
            ".revoke(",
        ] {
            assert!(
                !permission_helpers.contains(forbidden),
                "CLI permission helpers must not mutate SQLite directly via `{forbidden}`"
            );
        }
        let render = fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_render.rs"))
            .expect("read CLI permission rendering");
        for required in [
            "create_permission_rule_command(command)",
            "replace_permission_rule_command(args.rule_id, command)",
            "revoke_permission_rule_as(args.rule_id, args.reason, Some(\"cli\".to_owned()))",
            "list_permission_rules(",
        ] {
            assert!(
                render.contains(required),
                "CLI permission rendering must use application service `{required}`"
            );
        }
        assert!(
            !render.contains("permission_database("),
            "CLI must not reopen a raw Runtime database connection for permission commands"
        );
    }

    #[test]
    fn deprecated_plugin_tool_model_name_alias_is_deleted() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let registry =
            fs::read_to_string(workspace.join("crates/agena-plugin-host/src/registry.rs"))
                .expect("read plugin tool registry");
        assert!(!registry.contains("fn model_name("));
        assert!(!registry.contains("Legacy name retained for API compatibility"));
    }

    #[test]
    fn contract_crates_do_not_bypass_the_domain_layer() {
        for (from, to) in [
            ("agena-provider", "agena-tool"),
            ("agena-provider", "agena-storage"),
            ("agena-tool", "agena-provider"),
            ("agena-tool", "agena-storage"),
            ("agena-storage", "agena-provider"),
            ("agena-storage", "agena-tool"),
        ] {
            assert!(
                FORBIDDEN_EDGES.contains(&(from, to)),
                "missing rule {from} -> {to}"
            );
        }
    }

    #[test]
    fn provider_auth_values_have_no_core_definition_or_facade() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(
            !workspace
                .join("crates/agena-runtime/src/provider/auth/types.rs")
                .exists(),
            "core auth value definitions must remain deleted"
        );
        let auth_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/auth/mod.rs"))
                .expect("read core auth module");
        for value in [
            "AuthData",
            "CredentialIssuer",
            "OAuthUserInfo",
            "OAuthTokenResponse",
            "CopilotDeployment",
            "OAuthCallback",
        ] {
            assert!(
                !auth_module.contains(&format!("pub use agena_provider::{value}")),
                "core auth module must not re-export {value}"
            );
        }
    }

    #[test]
    fn tool_input_policy_values_have_no_core_definition() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tool_source = fs::read_to_string(workspace.join("crates/agena-tool/src/lib.rs"))
            .expect("read tool contract source");
        assert!(tool_source.contains("pub enum ReadMode"));
        assert!(tool_source.contains("pub struct TaskModelSelection"));
        assert!(tool_source.contains("pub struct ToolExecutionSummary"));
        assert!(tool_source.contains("pub struct ToolAttachmentSummary"));
        assert!(tool_source.contains("pub payload: Option<serde_json::Value>"));
        assert!(tool_source.contains("Runtime-neutral summary"));
        let result_source =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/tool/result.rs"))
                .expect("read core tool execution result projection");
        assert!(result_source.contains("pub fn summary(&self)"));
        assert!(result_source.contains("pub fn apply_neutral_fields"));
        assert!(result_source.contains("pub fn set_neutral_output"));
        assert!(result_source.contains("pub fn insert_neutral_metadata"));
        let truncation_source =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/tool/truncation.rs"))
                .expect("read tool truncation seam consumer");
        assert!(
            truncation_source.contains("set_neutral_output"),
            "tool truncation must apply output through the neutral seam"
        );
        assert!(
            !truncation_source.contains("execution.view.output_text ="),
            "tool truncation must not write the neutral output field directly"
        );
        assert!(
            result_source.matches("pub fn summary(&self)").count() >= 3,
            "core execution result types must expose neutral summary projections"
        );
        let hooks_source = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/tool/executor/executor_hooks.rs"),
        )
        .expect("read tool hook summary consumer");
        assert!(
            hooks_source.contains("let summary = execution.summary()"),
            "plugin tool hooks must consume the neutral execution summary"
        );
        assert!(
            hooks_source.contains("apply_neutral_fields"),
            "plugin tool hooks must apply presentation updates through the core boundary method"
        );
        let replies_source = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/session/manager/replies/replies_execution.rs"),
        )
        .expect("read session tool completion summary consumer");
        assert!(
            replies_source.contains("let summary = execution.summary()"),
            "session tool completion must consume the neutral execution summary"
        );
        let output_source =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/tool/output_helpers.rs"))
                .expect("read model output summary consumer");
        assert!(
            output_source.contains("let summary = execution.summary()"),
            "model output boundary must consume the neutral execution summary"
        );
        for (relative_path, label) in [
            (
                "crates/agena-api-server/src/rest/plugins.rs",
                "API plugin response",
            ),
            (
                "crates/agena-runtime/src/plugins/provided/router.rs",
                "in-process plugin router",
            ),
            (
                "crates/agena-runtime/src/runtime/host_client/mappers.rs",
                "host client mapper",
            ),
            ("crates/agena-cli/src/cli/mod.rs", "CLI MCP/prompt boundary"),
            ("crates/agena-cli/src/cli/cli_render.rs", "CLI renderer"),
        ] {
            let source = fs::read_to_string(workspace.join(relative_path))
                .unwrap_or_else(|_| panic!("read {label} summary consumer"));
            assert!(
                source.contains("let summary = execution.summary()")
                    || source.contains("execute_invocation_summary")
                    || source.contains("execute_session_tool")
                    || source.contains("execute_runtime_tool"),
                "{label} must consume the neutral execution summary"
            );
        }
        assert!(!hooks_source.contains("execution.output.to_json_payload()"));
        let cli_source = fs::read_to_string(workspace.join("crates/agena-cli/src/cli/mod.rs"))
            .expect("read CLI summary consumer");
        assert!(
            cli_source.contains("execution.summary().payload")
                || cli_source.contains("summary.payload")
        );
        assert!(!cli_source.contains("execution.output.to_json_payload()"));

        let core_tool_source =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/message/part/tool.rs"))
                .expect("read core tool input source");
        assert!(
            !workspace
                .join("crates/agena-runtime/src/tool_protocol.rs")
                .exists(),
            "deprecated tool_protocol facade must remain deleted"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/tool_api.rs")
                .exists(),
            "deprecated core tool_api facade must remain deleted"
        );
        assert!(!core_tool_source.contains("pub enum ReadMode"));
        assert!(!core_tool_source.contains("pub struct TaskModelSelection"));
        let message_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/message/mod.rs"))
                .expect("read core message facade");
        assert!(!message_module.contains("ReadMode"));
        assert!(!message_module.contains("TaskModelSelection"));
        assert!(!message_module.contains("FilesystemEffect"));
        assert!(!message_module.contains("NetworkEffect"));
        assert!(!message_module.contains("ExecutionStatusTransitionError"));
        assert!(!message_module.contains("PartStateTransitionError"));
        for value in [
            "ProcessEvent",
            "ProcessShell",
            "ProcessStatus",
            "ProcessStream",
            "ProcessSummary",
        ] {
            assert!(!message_module.contains(value));
        }
        let tool_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/tool/mod.rs"))
                .expect("read core tool facade");
        for value in [
            "BuiltinToolProfile",
            "ToolAvailability",
            "CronJobSummary",
            "AppliedFileChange",
            "ApplyPatchExecution",
            "PatchOpKind",
            "SnapshotBackend",
            "SnapshotBackendCapabilities",
            "SnapshotBackendSupport",
        ] {
            assert!(!tool_module.contains(&format!("pub use agena_tool::{value}")));
        }
        let catalog_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog/mod.rs"))
                .expect("read model catalog module");
        assert!(!catalog_module.contains("pub use agena_domain::{ModelPricing"));
        let config_types =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/config_values.rs"))
                .expect("read Runtime configuration values");
        assert!(!config_types.contains("ProviderNativeToolFreshness"));
        let config_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/config/mod.rs"))
                .expect("read config module");
        assert!(!config_module.contains("ProviderNativeToolFreshness"));
        assert!(!config_types.contains("ProviderNativeToolHarnessKind"));
        assert!(!config_module.contains("ProviderNativeToolHarnessKind"));
        assert!(!config_types.contains("ProviderNativeToolRoute"));
        assert!(!config_module.contains("ProviderNativeToolRoute"));
        assert!(!config_types.contains("ProviderNativeToolHarnessRef"));
        assert!(!config_module.contains("ProviderNativeToolHarnessRef"));
        assert!(!config_types.contains("ProviderNativeToolBinding"));
        assert!(!config_module.contains("ProviderNativeToolBinding"));
        assert!(!config_types.contains("ProviderNativeToolHarnessBindings"));
        assert!(!config_module.contains("ProviderNativeToolHarnessBindings"));
        for name in [
            "ProviderNativeToolsConfig",
            "ProviderHostedCodeExecutionConfig",
            "ProviderHostedFileSearchConfig",
            "ProviderHostedImageGenerationConfig",
            "ProviderHostedToolConfigs",
            "ProviderHostedUrlContextConfig",
            "ProviderHostedWebSearchConfig",
            "HostedCodeExecutionContainerConfig",
        ] {
            assert!(
                !config_types.contains(name),
                "config types facade still exports {name}"
            );
            assert!(
                !config_module.contains(name),
                "config facade still exports {name}"
            );
        }
        assert!(!config_types.contains("PluginsConfig as PluginConfig"));
        assert!(!config_module.contains("PluginsConfig as PluginConfig"));
        assert!(!config_types.contains("ProviderNativeToolKind"));
        assert!(!config_module.contains("ProviderNativeToolKind"));
        let provider_utils =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/utils.rs"))
                .expect("read provider utility module");
        assert!(!provider_utils.contains("pub use agena_provider"));
        let session_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/mod.rs"))
                .expect("read session module");
        assert!(!session_module.contains("ExecutionLifecycle"));
        assert!(!session_module.contains("ExecutionTransitionError"));
        assert!(!session_module.contains("SessionUsageLimitBasis"));
        assert!(!session_module.contains("SessionUsage,\n"));
        assert!(!session_module.contains("PromptTokenUsageSnapshot"));
        assert!(!session_module.contains("PromptCompactionActivity"));
        assert!(!session_module.contains("DoomLoopPolicy"));
        assert!(!session_module.contains("DoomLoopHit"));
        assert!(!session_module.contains("UsagePeriod,\n"));
        assert!(!session_module.contains("SessionCacheStats,\n"));
        assert!(!session_module.contains("SessionAutoCompactionConfig,\n"));
        let session_manager =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/manager/mod.rs"))
                .expect("read session manager module");
        assert!(!session_manager.contains("pub use agena_domain::SessionCacheStats"));
        let storage_contract =
            fs::read_to_string(workspace.join("crates/agena-storage/src/lib.rs"))
                .expect("read storage contract");
        assert!(!storage_contract.contains("sea_orm"));
        assert!(!storage_contract.contains("DatabaseTransaction"));

        let attachment_module = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/message/part/attachment.rs"),
        )
        .expect("read attachment boundary");
        assert!(
            attachment_module.contains("pub use agena_plugin_host::sdk::attachment::{"),
            "attachments must remain the plugin SDK-owned host boundary"
        );
        for value in [
            "AttachmentItem",
            "AttachmentKind",
            "AttachmentPart",
            "AttachmentSource",
        ] {
            assert!(
                !attachment_module.contains(&format!("pub enum {value}"))
                    && !attachment_module.contains(&format!("pub struct {value}")),
                "core must not duplicate plugin attachment value {value}"
            );
        }
        assert!(
            !workspace
                .join("crates/agena-runtime/src/message/usage.rs")
                .exists(),
            "Core must not retain a provider usage compatibility module"
        );
        let sqlite_usage = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/message_projection_repository.rs"),
        )
        .expect("read SQLite usage persistence boundary");
        assert!(sqlite_usage.contains("pub struct PersistedCompletionUsage"));
        assert!(sqlite_usage.contains("FromJsonQueryResult"));
    }

    #[test]
    fn legacy_runtime_shims_are_removed() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in [
            "crates/agena-runtime/src/runtime/background_tasks.rs",
            "crates/agena-runtime/src/runtime/store.rs",
            "crates/agena-runtime/src/runtime/plugin_slot.rs",
            "crates/agena-runtime/src/runtime/event_bridge.rs",
            "crates/agena-runtime/src/runtime/janitor.rs",
        ] {
            assert!(
                !workspace.join(relative).exists(),
                "legacy runtime shim must remain deleted: {relative}"
            );
        }
        assert!(
            workspace
                .join("crates/agena-runtime/src/event/bridge.rs")
                .exists(),
            "core event bridge must live at the event boundary"
        );
        let runtime_maintenance =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session_maintenance.rs"))
                .expect("read Runtime session maintenance loop");
        assert!(
            runtime_maintenance.contains("pub async fn run_session_maintenance<I, F, Fut>(")
                && runtime_maintenance.contains("run_periodic(")
                && !runtime_maintenance.contains("SessionManager"),
            "Runtime must own session maintenance scheduling without depending on Core session types"
        );
        let reload =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/reload.rs"))
                .expect("read core reload adapter");
        assert!(reload.contains("agena_runtime::run_reload_watch_loop"));
        assert!(!reload.contains("capture_watch_path_stamps"));
        assert!(!reload.contains("diff_watch_path_stamps"));
        let run_buffer = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/session/history/run_buffer.rs"),
        )
        .expect("read run buffer status boundary");
        assert!(!run_buffer.contains("crate::message::ExecutionStatus"));
        let history_store =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/history/store.rs"))
                .expect("read history store status boundary");
        assert!(!history_store.contains("crate::message::ExecutionStatus"));
        let history_event =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/history/event.rs"))
                .expect("read history event status boundary");
        assert!(!history_event.contains("message::{\n        ExecutionStatus"));
        let manager_history = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/session/manager/history.rs"),
        )
        .expect("read session manager history status boundary");
        assert!(!manager_history.contains("message::{ExecutionStatus"));
        assert!(
            !workspace
                .join("crates/agena-runtime/src/message/part/common.rs")
                .exists()
        );
        let event_client =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/event/client.rs"))
                .expect("read event client status boundary");
        assert!(!event_client.contains("message::{ExecutionStatus"));
        assert!(!manager_history.contains("message::{ExecutionStatus"));
        for relative in [
            "crates/agena-runtime/src/session/model.rs",
            "crates/agena-runtime/src/session/processor.rs",
            "crates/agena-runtime/src/session/manager/mod.rs",
        ] {
            let source =
                fs::read_to_string(workspace.join(relative)).expect("read session status boundary");
            assert!(
                source.contains("ExecutionStatus"),
                "session aggregate must import status from domain: {relative}"
            );
        }
        let runtime_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/mod.rs"))
                .expect("read core runtime module");
        for module in ["background_tasks", "store", "plugin_slot"] {
            assert!(
                !runtime_module.contains(&format!("mod {module}")),
                "core runtime module must not restore the {module} shim"
            );
        }
        for relative in [
            "crates/agena-runtime/src/provider/core.rs",
            "crates/agena-runtime/src/provider/tool_mode.rs",
            "crates/agena-runtime/src/provider/multi_adapter.rs",
            "crates/agena-runtime/src/provider/cataloged_models.rs",
            "crates/agena-runtime/src/provider/registry/listing.rs",
            "crates/agena-runtime/src/provider/registry/completion.rs",
        ] {
            let source = fs::read_to_string(workspace.join(relative))
                .expect("read provider tool mode source");
            assert!(
                !source.contains("config::AgenaToolMode"),
                "provider runtime must consume the provider-owned tool mode: {relative}"
            );
        }
    }

    #[test]
    fn compaction_bounds_are_runtime_owned() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let runtime_policy =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/compaction_policy.rs"))
                .expect("read Runtime compaction policy");
        for required in [
            "pub const MAX_RECENT_USER_TURNS: usize = 2",
            "pub const MAX_RECENT_CONTEXT_CHARS: usize = 32_000",
            "pub const MAX_COMPACTOR_MESSAGE_CHARS: usize = 8_000",
            "pub const DEFAULT_COMPACTION_OUTPUT_TOKENS: u32 = 4_096",
            "pub const MAX_COMPACTION_FAILURES: u8 = 3",
        ] {
            assert!(
                runtime_policy.contains(required),
                "missing Runtime compaction policy: {required}"
            );
        }
        let core_compact = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/session/manager/compact.rs"),
        )
        .expect("read Core compaction adapter");
        assert!(core_compact.contains("use agena_runtime::{"));
        for forbidden in [
            "const MAX_RECENT_USER_TURNS",
            "const MAX_RECENT_CONTEXT_CHARS",
            "const MAX_COMPACTOR_MESSAGE_CHARS",
            "const DEFAULT_COMPACTION_OUTPUT_TOKENS",
            "const MAX_COMPACTION_FAILURES",
        ] {
            assert!(
                !core_compact.contains(forbidden),
                "Core must not own compaction bound: {forbidden}"
            );
        }
    }

    #[test]
    fn model_catalog_service_uses_provider_source_contract() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let service =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog_service.rs"))
                .expect("read runtime model catalog service");
        assert!(service.contains("ProviderModelSource"));
        assert!(!service.contains("ProviderRegistry"));
        assert!(service.contains("ModelCatalogRepository"));
        assert!(
            !service.contains("ModelCatalogStore"),
            "catalog service must not depend on the concrete SeaORM store"
        );
    }

    #[test]
    fn provider_error_classification_is_provider_owned() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let provider = fs::read_to_string(workspace.join("crates/agena-provider/src/lib.rs"))
            .expect("read provider contract crate");
        assert!(provider.contains("pub enum ProviderErrorKind"));

        let core_error = fs::read_to_string(workspace.join("crates/agena-runtime/src/error.rs"))
            .expect("read core error module");
        assert!(!core_error.contains("pub enum ProviderErrorKind"));

        for relative in [
            "crates/agena-runtime/src/provider",
            "crates/agena-runtime/src/session/manager",
        ] {
            let source = collect_rust_sources(&workspace.join(relative))
                .expect("read provider error consumers")
                .join("\n");
            assert!(
                !source.contains("crate::error::ProviderErrorKind"),
                "provider error consumers must use the provider contract: {relative}"
            );
        }
    }

    #[test]
    fn system_notice_kind_is_domain_owned() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let domain = fs::read_to_string(workspace.join("crates/agena-domain/src/system_notice.rs"))
            .expect("read system notice domain value");
        assert!(domain.contains("pub enum SystemNoticeKind"));

        let history_event =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/history/event.rs"))
                .expect("read history event module");
        assert!(history_event.contains("SystemNoticeKind"));
        assert!(!history_event.contains("pub enum SystemNoticeKind"));

        let history_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/history/mod.rs"))
                .expect("read history module");
        assert!(!history_module.contains("SystemNoticeKind,"));
    }

    #[test]
    fn agent_scope_is_domain_owned() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let domain = fs::read_to_string(workspace.join("crates/agena-domain/src/agent_scope.rs"))
            .expect("read agent scope domain value");
        assert!(domain.contains("pub enum AgentScope"));

        let agents = fs::read_to_string(workspace.join("crates/agena-runtime/src/agents/mod.rs"))
            .expect("read core agent registry");
        assert!(!agents.contains("pub enum AgentScope"));

        let application = collect_rust_sources(&workspace.join("crates/agena-application/src"))
            .expect("read application sources")
            .join("\n");
        assert!(application.contains("agena_domain::AgentScope"));
        assert!(!application.contains("agena::agents::AgentScope"));
    }

    #[test]
    fn interactive_request_schema_boundary_stays_explicit() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let activity =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/message/part/activity.rs"))
                .expect("read interactive request message values");
        assert!(activity.contains("pub enum RequestPart"));
        assert!(!activity.contains("pub enum PendingInteractiveRequest"));
        assert!(!activity.contains("pub struct UserInputQuestion"));
        assert!(!activity.contains("pub struct UserInputOption"));
        assert!(!activity.contains("pub struct UserInputRequest"));
        assert!(!activity.contains("pub struct UserInputReply"));
        let tool =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/message/part/tool.rs"))
                .expect("read ask-user tool input");
        assert!(tool.contains("ToolInput"));

        let domain = fs::read_to_string(workspace.join("crates/agena-domain/src/user_input.rs"))
            .expect("read domain user-input values");
        assert!(domain.contains("PendingInteractiveRequestKind"));
        assert!(domain.contains("UserInputReplyKind"));
        assert!(!domain.contains("ToolInput"));
        let activity_domain =
            fs::read_to_string(workspace.join("crates/agena-domain/src/message_activity.rs"))
                .expect("read domain user-input question values");
        assert!(activity_domain.contains("pub struct UserInputQuestion"));
        assert!(activity_domain.contains("pub struct UserInputOption"));
        assert!(activity_domain.contains("pub struct UserInputRequest"));
        assert!(activity_domain.contains("pub struct UserInputReply"));
        assert!(!activity_domain.contains("enum PendingInteractiveRequest"));
        let pending = fs::read_to_string(
            workspace.join("crates/agena-domain/src/pending_interactive_request.rs"),
        )
        .expect("read domain pending interactive request");
        assert!(pending.contains("pub enum PendingInteractiveRequest"));
    }

    #[test]
    fn upper_layers_do_not_use_the_core_model_catalog_facade() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (relative_path, label) in [
            ("apps/agena/src", "terminal application"),
            ("crates/agena-api-server/src", "API server"),
        ] {
            let source = collect_rust_sources(&workspace.join(relative_path))
                .unwrap_or_else(|_| panic!("read {label} sources"))
                .join("\n");
            assert!(
                !source.contains("agena::model_catalog"),
                "{label} must use provider/runtime catalog contracts rather than the core facade"
            );
        }
    }

    #[test]
    fn upper_layers_consume_declarative_permission_values_from_domain() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (relative_path, label) in [
            ("apps/agena/src", "terminal application"),
            ("crates/agena-application/src", "application services"),
        ] {
            let source = collect_rust_sources(&workspace.join(relative_path))
                .unwrap_or_else(|_| panic!("read {label} sources"))
                .join("\n");
            for forbidden in [
                "agena::agent::PermissionConfig",
                "agena::agent::AgentPermissionConfig",
                "agena::agent::PathAccessModes",
                "agena::agent::PathAccessRuleConfig",
                "agena::agent::ToolPermissionRules",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{label} must consume the domain permission value instead of `{forbidden}`"
                );
            }
        }
    }

    #[test]
    fn transaction_bound_permission_rule_writes_use_a_storage_contract() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let storage = fs::read_to_string(workspace.join("crates/agena-storage/src/lib.rs"))
            .expect("read storage contracts");
        assert!(
            storage.contains("trait PermissionRuleTransactionWriter<Transaction>"),
            "storage must own the generic transaction-bound permission-rule writer contract"
        );
        assert!(
            !storage.contains("DatabaseTransaction"),
            "storage transaction contract must not expose SeaORM transaction types"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/db/sea_permission_rule_repository.rs")
                .exists(),
            "core must not retain the SQLite transaction-bound permission adapter"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/db/crud/permission_rule.rs")
                .exists(),
            "core must not retain regular permission-rule CRUD beside its transaction writer"
        );
        let sqlite_adapter = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/permission_rule_repository.rs"),
        )
        .expect("read SQLite permission-rule repository");
        assert!(
            sqlite_adapter
                .contains("impl PermissionRuleRepository for SeaPermissionRuleRepository"),
            "SQLite storage must own the regular permission-rule repository"
        );
        assert!(
            sqlite_adapter.contains("impl PermissionRuleTransactionWriter<DatabaseTransaction> for SeaPermissionRuleTransactionWriter"),
            "SQLite storage must own the transaction-bound permission writer"
        );
        assert!(
            !sqlite_adapter.contains("crud::permission_rule"),
            "transaction writer must own its limited transaction SQL rather than depend on generic CRUD"
        );
        let history =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/store/history.rs"))
                .expect("read session persistence choreography");
        let production_history = history
            .split("#[cfg(test)]")
            .next()
            .expect("production session persistence choreography");
        assert!(
            production_history
                .contains("permission_rule_transaction_writer\n                            .upsert_in_transaction"),
            "session persistence must use its injected transaction writer"
        );
        assert!(
            !production_history.contains("SeaPermissionRuleRepository::upsert_in_transaction"),
            "session persistence must not statically call the SeaORM repository"
        );
    }

    #[test]
    fn usage_repository_is_owned_by_sqlite_storage() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(
            workspace
                .join("crates/agena-storage-sqlite/src/usage_repository.rs")
                .is_file(),
            "SQLite storage must own the concrete usage repository"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/db/sea_usage_repository.rs")
                .exists(),
            "core must not retain a concrete usage repository"
        );
        let manager =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/manager/mod.rs"))
                .expect("read session manager composition");
        assert!(
            manager.contains("agena_storage_sqlite::SeaUsageRepository"),
            "core composition must select the SQLite usage adapter"
        );
    }

    #[test]
    fn workspace_repository_is_owned_by_sqlite_storage() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(
            workspace
                .join("crates/agena-storage-sqlite/src/workspace_repository.rs")
                .is_file(),
            "SQLite storage must own the concrete workspace repository"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/db/crud/workspace.rs")
                .exists(),
            "core must not retain concrete workspace CRUD"
        );
        let core_sources = collect_rust_sources(&workspace.join("crates/agena-runtime/src"))
            .expect("read runtime concrete-adapter sources")
            .join("\n");
        assert!(
            !core_sources.contains("crud::workspace"),
            "core must consume the workspace storage port rather than workspace CRUD"
        );
    }

    #[test]
    fn projected_message_reads_are_owned_by_sqlite_storage() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let storage = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/message_projection_repository.rs"),
        )
        .expect("read SQLite message projection repository");
        assert!(storage.contains("SeaMessageProjectionRepository"));
        assert!(storage.contains("impl MessageProjectionRepository"));
        let history =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/history/store.rs"))
                .expect("read session history store");
        assert!(
            history.contains("message_projection_repository")
                && history.contains(".list_headers(")
                && history.contains(".list_parts(message_ids, include_full_parts)"),
            "Runtime must synchronize projections then delegate visible header and part reads to storage"
        );
        assert!(
            !history.contains("list_projected_message_rows")
                && !history.contains("list_projected_message_rows_page")
                && !history.contains("projected_message_from_row")
                && !history.contains("projected_metadata_from_row"),
            "Core must not restore entity-backed projected message readers"
        );
        assert!(
            !history.contains("legacy_load_projected_parts_for_messages_from_entities"),
            "Core must not retain an entity-backed projected-part read fallback"
        );
    }

    #[test]
    fn projected_message_writes_are_owned_by_sqlite_transaction_writer() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let storage = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/message_projection_repository.rs"),
        )
        .expect("read SQLite message projection writer");
        assert!(storage.contains("SeaMessageProjectionTransactionWriter"));
        assert!(storage.contains("impl MessageProjectionTransactionWriter<DatabaseTransaction>"));
        assert!(storage.contains("terminalize_open_messages_in_transaction"));
        assert!(storage.contains("clear_session_projection_in_transaction"));
        assert!(storage.contains("upsert_projection_watermark_in_transaction"));

        let history =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/history/store.rs"))
                .expect("read session history store");
        assert!(history.contains("message_projection_transaction_writer"));
        assert!(history.contains("TransactionProjectionPartWriter"));
        assert!(history.contains("#[cfg(test)]\nasync fn upsert_message_projection"));
        assert!(history.contains("#[cfg(test)]\nasync fn upsert_part_projection"));
        assert!(history.contains("#[cfg(test)]\nasync fn terminalize_open_messages"));
        assert!(history.contains("#[cfg(test)]\nasync fn clear_projection_for_session"));
        assert!(history.contains("#[cfg(test)]\nasync fn upsert_projection_state"));
        assert!(history.contains(".upsert_projection_watermark("));
        assert!(history.contains(".clear_session_projection("));
        assert!(history.contains(".terminalize_open_messages("));
    }

    #[test]
    fn transcript_reads_use_runtime_projection_without_session_manager() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dispatch =
            fs::read_to_string(workspace.join("crates/agena-application/src/dispatch/queries.rs"))
                .expect("read application query dispatch");
        assert!(!dispatch.contains("session_manager()"));

        let messages =
            fs::read_to_string(workspace.join("crates/agena-application/src/service/messages.rs"))
                .expect("read application message service");
        assert!(messages.contains("load_visible_message_projection_from_queries"));
        assert!(messages.contains(".list_projected_messages(session_id, include_content)"));
        assert!(messages.contains("if query.parts == PartLoadMode::Summary"));
        assert!(messages.contains("if parts == PartLoadMode::Summary"));
        assert!(messages.contains("if mode == PartLoadMode::Summary"));
        assert!(messages.contains("find_session_id_for_part(part_id)"));
        assert!(!messages.contains("SessionManager"));
    }

    #[test]
    fn simple_transcript_detail_values_are_typed_at_runtime_boundary() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let runtime =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session_query_service.rs"))
                .expect("read runtime session query service");
        assert!(runtime.contains("pub enum SessionProjectedPartDetail"));
        for variant in [
            "Text {",
            "Reasoning {",
            "Error {",
            "Attachment(agena_plugin_host::sdk::attachment::AttachmentPart)",
            "PermissionRequest {",
            "UserInputRequest {",
            "Operation(Box<SessionProjectedOperationPart>)",
            "Opaque(serde_json::Value)",
        ] {
            assert!(runtime.contains(variant));
        }

        let application =
            fs::read_to_string(workspace.join("crates/agena-application/src/service/messages.rs"))
                .expect("read application message service");
        for variant in [
            "SessionProjectedPartDetail::Text",
            "SessionProjectedPartDetail::Reasoning",
            "SessionProjectedPartDetail::Error",
            "SessionProjectedPartDetail::Attachment",
            "SessionProjectedPartDetail::PermissionRequest",
            "SessionProjectedPartDetail::UserInputRequest",
            "SessionProjectedPartDetail::Operation",
        ] {
            assert!(application.contains(variant));
        }
        assert!(application.contains("matches!(detail, SessionProjectedPartDetail::Opaque(_))"));
        for forbidden in [
            "use agena::message::{MessagePart, PartContent}",
            "fn part_content_from_runtime_projection",
            "fn operation_part_from_runtime_projection",
            "agena::message::OperationPart",
            "agena::message::OperationBlock",
            "agena::message::ToolResultEnvelope",
        ] {
            assert!(
                !application.contains(forbidden),
                "Application transcript projection must not reconstruct Core value: {forbidden}"
            );
        }
        assert!(runtime.contains("pub enum SessionProjectedOperationBlock"));
        assert!(runtime.contains("pub struct SessionProjectedToolResult"));
        assert!(runtime.contains("pub struct SessionProjectedOperationPart"));
        assert!(application.contains("agena_plugin_host::sdk::attachment::AttachmentItem"));
        for forbidden in [
            "agena::message::AttachmentItem",
            "agena::message::AttachmentKind",
            "agena::message::AttachmentSource",
        ] {
            assert!(
                !application.contains(forbidden),
                "application attachment projection must use the plugin SDK, not Core alias `{forbidden}`"
            );
        }
    }

    #[test]
    fn completion_usage_is_canonical_and_sqlite_owns_its_orm_wrapper() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let provider = fs::read_to_string(workspace.join("crates/agena-provider/src/lib.rs"))
            .expect("read provider values");
        assert!(provider.contains("pub struct CompletionUsage"));
        assert!(provider.contains("impl CompletionUsage"));

        assert!(
            !workspace
                .join("crates/agena-runtime/src/message/usage.rs")
                .exists(),
            "Core must not retain a CompletionUsage compatibility facade"
        );

        let sqlite = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/message_projection_repository.rs"),
        )
        .expect("read sqlite message projection repository");
        assert!(sqlite.contains("pub struct PersistedCompletionUsage"));
        assert!(sqlite.contains("FromJsonQueryResult"));
    }

    #[test]
    fn persisted_event_queries_cross_the_runtime_projection_boundary() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let runtime =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/event_query_service.rs"))
                .expect("read runtime event query service");
        for required in [
            "pub struct RuntimeEvent",
            "pub trait RuntimeEventQueryService",
            "pub trait RuntimeEventStreamService",
            "pub enum RuntimeLiveEventSubscriptionItem",
            "async fn list_events(",
            "async fn list_events_before(",
        ] {
            assert!(runtime.contains(required));
        }
        let runtime_publish =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/event_publish_service.rs"))
                .expect("read runtime event publish service");
        assert!(runtime_publish.contains("pub trait RuntimeEventPublishService"));
        let runtime_application_services =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/application_services.rs"))
                .expect("read runtime application service bundle");

        let application =
            fs::read_to_string(workspace.join("crates/agena-application/src/application.rs"))
                .expect("read application composition");
        assert!(application.contains("RuntimeEventQueryService"));
        assert!(application.contains("RuntimeEventStreamService"));
        assert!(
            application.contains("event_publisher")
                && runtime_application_services.contains("RuntimeEventPublishService"),
            "application event publication must arrive through the runtime-owned builder result"
        );
        assert!(!application.contains("EventStore<agena::event::EventKind>"));
        assert!(!application.contains("EventBus<agena::event::EventKind>"));
        assert!(!application.contains("ApplicationEventPublisher"));

        for relative in [
            "crates/agena-application/src/service/sessions.rs",
            "crates/agena-application/src/service/execution.rs",
            "crates/agena-application/src/dispatch/queries.rs",
        ] {
            let source = fs::read_to_string(workspace.join(relative))
                .expect("read application event query consumer");
            assert!(
                source.contains("RuntimeEventQueryService") || source.contains("RuntimeEventRange")
            );
            assert!(!source.contains("EventStore<agena::event::EventKind>"));
        }

        let projection =
            fs::read_to_string(workspace.join("crates/agena-application/src/event_projection.rs"))
                .expect("read application event projection");
        assert!(projection.contains("event_resource_from_runtime"));
        assert!(!projection.contains("&agena::event::DomainEvent"));

        let permissions = fs::read_to_string(
            workspace.join("crates/agena-application/src/service/permissions.rs"),
        )
        .expect("read application permission event publisher");
        assert!(permissions.contains("RuntimeEventPublishRequest::PermissionRuleCreated"));
        assert!(permissions.contains("RuntimeEventPublishRequest::PermissionRuleUpdated"));
        assert!(permissions.contains("RuntimeEventPublishRequest::PermissionRuleRevoked"));
        assert!(!permissions.contains("EventKind::PermissionRule"));
    }

    #[test]
    fn session_summary_repository_is_owned_by_sqlite_storage() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(
            workspace
                .join("crates/agena-storage-sqlite/src/session_summary_repository.rs")
                .is_file(),
            "SQLite storage must own the concrete session-summary repository"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/db/sea_session_summary_repository.rs")
                .exists(),
            "core must not retain a concrete session-summary repository"
        );
        let manager =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/manager/mod.rs"))
                .expect("read session manager composition");
        assert!(
            manager.contains("agena_storage_sqlite::SeaSessionSummaryRepository"),
            "core composition must select the SQLite session-summary adapter"
        );
    }

    #[test]
    fn event_store_is_owned_by_sqlite_storage() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(
            workspace
                .join("crates/agena-storage-sqlite/src/event_store.rs")
                .is_file(),
            "SQLite storage must own the concrete generic event store"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/db/sea_event_store.rs")
                .exists(),
            "core must not retain a concrete Sea event store"
        );
        let manager =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/manager/mod.rs"))
                .expect("read session manager composition");
        assert!(
            manager.contains("agena_storage_sqlite::SeaEventStore"),
            "core composition must select the SQLite event-store adapter"
        );
    }

    #[test]
    fn sqlite_active_enums_are_not_core_owned() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let storage =
            fs::read_to_string(workspace.join("crates/agena-storage-sqlite/src/stored_values.rs"))
                .expect("read SQLite active enum definitions");
        for name in ["StoredRole", "StoredExecutionStatus", "StoredPartKind"] {
            assert!(storage.contains(name), "SQLite storage must own {name}");
        }
        for file in [
            "stored_role.rs",
            "stored_execution_status.rs",
            "stored_part_kind.rs",
        ] {
            assert!(
                !workspace
                    .join("crates/agena-runtime/src/db")
                    .join(file)
                    .exists(),
                "core must not retain {file}"
            );
        }
    }

    #[test]
    fn sqlite_schema_lifecycle_is_owned_by_storage() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let lifecycle = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/schema_lifecycle.rs"),
        )
        .expect("read SQLite schema lifecycle");
        assert!(lifecycle.contains("begin_schema_initialization"));
        assert!(lifecycle.contains("complete_schema_initialization"));
        assert!(lifecycle.contains("CURRENT_SCHEMA_VERSION"));
        let core_schema = workspace.join("crates/agena-runtime/src/db/schema.rs");
        assert!(
            !core_schema.exists(),
            "core must not retain concrete SQLite schema definitions"
        );
        assert!(
            workspace
                .join("crates/agena-storage-sqlite/src/schema.rs")
                .is_file(),
            "SQLite storage must own concrete tables and indexes"
        );
        let invariants = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/schema_invariants.rs"),
        )
        .expect("read SQLite schema invariants");
        assert!(invariants.contains("CREATE TRIGGER"));
    }

    #[test]
    fn runtime_owns_public_catalog_source_enablement_policy() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let runtime_source =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog_source.rs"))
                .expect("read runtime catalog source policy");
        assert!(
            runtime_source.contains("pub fn public_model_catalog_sources_enabled()"),
            "runtime must own the public catalog source enablement policy"
        );
        let core_catalog =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog/mod.rs"))
                .expect("read core catalog facade");
        assert!(
            !core_catalog.contains("AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES"),
            "core catalog adapter must not own the public source environment policy"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/model_catalog/sources.rs")
                .exists(),
            "core must not retain the public catalog source adapter"
        );
        let runtime_sources =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog_source.rs"))
                .expect("read runtime default source list");
        assert!(
            runtime_sources.contains("pub fn default_public_model_catalog_sources()"),
            "runtime must own the executable default public catalog source list"
        );
        assert!(
            runtime_sources.contains("pub fn merge_public_model_catalog_documents("),
            "runtime must own public-source ordering and merge composition"
        );
        assert!(
            runtime_sources.contains("pub async fn fetch_public_model_catalog_documents("),
            "runtime must own public-source collection and warning aggregation"
        );
        assert!(
            runtime_sources.contains("pub struct ModelCatalogConfiguredPublicSource"),
            "runtime must own public-source composition over a concrete fetcher"
        );
    }

    #[test]
    fn provider_owns_pure_catalog_baseline_decoration() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let provider =
            fs::read_to_string(workspace.join("crates/agena-provider/src/catalog_decoration.rs"))
                .expect("read provider catalog decoration");
        let provider_model_decoration = fs::read_to_string(
            workspace.join("crates/agena-provider/src/catalog_model_decoration.rs"),
        )
        .expect("read provider model catalog decoration");
        for required in [
            "pub fn apply_configured_definition_as_baseline(",
            "pub fn merge_catalog_baseline_thinking_modes(",
            "pub fn merge_catalog_baseline_speed_modes(",
        ] {
            assert!(
                provider.contains(required),
                "provider must own pure catalog decoration helper `{required}`"
            );
        }
        for required in [
            "pub trait CatalogModelDecorationSource",
            "pub fn decorate_provider_models(",
        ] {
            assert!(
                provider_model_decoration.contains(required),
                "provider must own catalog model decoration through `{required}`"
            );
        }
        let core = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/model_catalog/decorate.rs"),
        )
        .expect("read core catalog decoration adapter");
        for forbidden in [
            "trait CatalogBaselineMode",
            "fn apply_configured_definition_as_baseline(",
            "fn merge_catalog_baseline_thinking_modes(",
            "fn merge_catalog_baseline_speed_modes(",
        ] {
            assert!(
                !core.contains(forbidden),
                "core must not restore pure catalog decoration helper `{forbidden}`"
            );
        }
        assert!(
            core.contains("impl agena_provider::CatalogModelDecorationSource"),
            "core catalog decoration may retain only the ModelRuntime adapter"
        );
        let snapshot =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/snapshot/mod.rs"))
                .expect("read runtime snapshot catalog decoration call site");
        assert!(
            snapshot
                .contains("use agena_provider::{ModelCatalogResponse, decorate_provider_models};")
                && snapshot.contains("Ok(decorate_provider_models("),
            "runtime snapshot must call the provider-owned catalog decoration algorithm directly"
        );
    }

    #[test]
    fn snapshot_facade_delegates_concrete_service_construction() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let snapshot =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/snapshot/mod.rs"))
                .expect("read runtime snapshot facade");
        for forbidden in [
            "build_provider_registry_with_plugins_and_catalog",
            "config_from_plugins(&resolution.config.plugins)",
            "SubagentRegistry::discover",
            "SessionManager::new",
            "spawn_event_bridge(",
            "install_plugin_host(",
        ] {
            assert!(
                !snapshot.contains(forbidden),
                "snapshot facade must delegate concrete construction: {forbidden}"
            );
        }
        assert!(
            snapshot.contains("agena_runtime::RuntimeServiceBundle::new"),
            "snapshot must construct the runtime-owned generic service bundle directly"
        );
        assert!(
            snapshot.contains("database.as_ref().map(|db|"),
            "snapshot must own the optional database-to-session composition"
        );
        let builders = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime/snapshot/builders.rs"),
        )
        .expect("read runtime snapshot builders");
        assert!(
            !builders.contains("pub(super) fn build_session_service"),
            "legacy core must not retain a session-service passthrough wrapper"
        );
    }

    #[test]
    fn snapshot_boundary_factories_consume_projected_plugin_inputs() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let builders = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime/snapshot/builders.rs"),
        )
        .expect("read runtime snapshot builders");
        assert!(!builders.contains("fn build_mcp_manager("));
        let runtime_mcp =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/mcp_runtime.rs"))
                .expect("read runtime MCP composition");
        assert!(runtime_mcp.contains("pub fn mcp_config_from_plugins("));
        assert!(runtime_mcp.contains("pub async fn build_configured_mcp_manager("));
        let core_mcp =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/plugins/provided/mcp.rs"))
                .expect("read core MCP plugin implementation");
        for forbidden in [
            "pub(crate) struct McpConfig",
            "pub(crate) struct McpRuntimeConfig",
            "pub(crate) struct McpTokenStoreConfig",
            "pub(crate) enum McpServerConfig",
            "pub(crate) struct McpStdioProcessConfig",
            "pub(crate) struct McpHttpEndpointConfig",
            "pub(crate) enum McpHttpAuthConfig",
            "pub(crate) fn config_from_plugins(",
            "pub(crate) fn static_bridge_enabled(",
            "pub(crate) async fn build_manager(",
        ] {
            assert!(
                !core_mcp.contains(forbidden),
                "Core MCP plugin must not restore Runtime-owned configuration/composition: {forbidden}"
            );
        }
        let runtime_lsp =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/lsp_config.rs"))
                .expect("read runtime LSP configuration");
        assert!(runtime_lsp.contains("pub struct LspConfig"));
        assert!(runtime_lsp.contains("pub fn lsp_config_from_plugins("));
        let core_lsp =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/plugins/provided/lsp.rs"))
                .expect("read core LSP plugin implementation");
        for forbidden in [
            "pub(crate) const LSP_PLUGIN_ID",
            "pub(crate) struct LspConfig",
            "pub(crate) struct LspServerDefaultsConfig",
            "pub(crate) struct LspServerConfig",
            "pub(crate) struct LspServerProcessConfig",
            "pub(crate) struct LspServerRoutingConfig",
            "pub(crate) struct LspServerSessionConfig",
            "pub(crate) fn config_from_plugins(",
        ] {
            assert!(
                !core_lsp.contains(forbidden),
                "Core LSP plugin must not restore Runtime-owned configuration: {forbidden}"
            );
        }
        assert!(!builders.contains("fn build_lsp_registry("));
        assert!(!builders.contains("fn build_lsp_services("));
        assert!(runtime_lsp.contains("pub fn compose_lsp_services("));
        let runtime_registration =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/registration.rs"))
                .expect("read Runtime configured-agent registration projection");
        assert!(runtime_registration.contains("pub struct RuntimeAgentRegistration"));
        assert!(runtime_registration.contains("pub fn configured_agent_registrations("));
        assert!(builders.contains(
            "pub(super) fn build_agent_registry(\n    workspace_root: &Path,\n    config_parent: Option<&Path>"
        ));
        let runtime_policy =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/policy.rs"))
                .expect("read Runtime scheduling policy");
        assert!(runtime_policy.contains("scheduler_poll_interval: Duration::from_secs(10)"));
        assert!(!builders.contains("Duration::from_secs(10)"));
        assert!(builders.contains("agena_runtime::compose_scheduler("));
        assert!(!builders.contains("build_in_memory("));
        let runtime_scheduler =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/scheduler_composition.rs"))
                .expect("read Runtime scheduler composition");
        assert!(
            runtime_scheduler.contains("pub fn compose_scheduler<S>(")
                && runtime_scheduler.contains("build_in_memory(")
                && runtime_scheduler
                    .contains("RuntimeSchedulingPolicy::default().scheduler_poll_interval")
        );
        let runtime_composition_helpers =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/composition.rs"))
                .expect("read Runtime session composition helpers");
        assert!(
            runtime_composition_helpers
                .contains("pub(crate) fn session_build_config_from_resolved(")
        );
        assert!(
            builders.contains("agena_runtime::configured_agent_registrations(agents)")
                && runtime_composition_helpers
                    .contains("pub(crate) fn session_build_config_from_resolved(")
        );
        let core_session_manager =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/manager/mod.rs"))
                .expect("read Core session-manager config adapter");
        assert!(core_session_manager.contains("use agena_runtime::RuntimeSessionManagerConfig;"));
        assert!(!core_session_manager.contains("pub type SessionManagerConfig"));
        assert!(!core_session_manager.contains("pub struct SessionManagerConfig"));
        assert!(!core_session_manager.contains("DEFAULT_MAX_CONCURRENT_TOOLS"));
        assert!(builders.contains("pub(super) fn build_or_reconfigure_session_manager("));
        assert!(builders.contains(
            "type SessionCompositionInputs<'a> = agena_runtime::SessionCompositionInputs"
        ));
        assert!(builders.contains("inputs: SessionCompositionInputs<'_>"));
        assert!(builders.contains(
            "pub(super) fn build_tool_executor(\n    inputs: agena_runtime::ToolCompositionInputs"
        ));
        assert!(builders.contains(
            "pub(super) async fn build_plugin_services(\n    inputs: agena_runtime::PluginCompositionInputs"
        ));
        assert!(builders.contains("agena_runtime::compose_and_install_plugin_host("));
        assert!(builders.contains("agena_runtime::codex_package_version()"));
        assert!(!builders.contains("env!(\"CARGO_PKG_VERSION\")"));
        let core_tool_registry =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/tool/tool_registry.rs"))
                .expect("read Core tool registry");
        assert!(!core_tool_registry.contains("default_tool_host("));
        assert!(!core_tool_registry.contains("PluginHostBuildConfig"));
        let runtime_plugin_composition =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/plugin_composition.rs"))
                .expect("read runtime PluginHost composition");
        for required in [
            "pub async fn compose_plugin_host(",
            "PluginHostBuildConfig::previous_plugins",
            "PluginHost::new(PluginHostBuildConfig {",
        ] {
            assert!(
                runtime_plugin_composition.contains(required),
                "Runtime must own PluginHost composition policy: {required}"
            );
        }
        let core_plugin_registry = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/config/registry/plugin_host.rs"),
        )
        .expect("read Core PluginHost registry adapter");
        for forbidden in ["fn build_plugin_host_from_inputs(", "PluginHostBuildConfig"] {
            assert!(
                !core_plugin_registry.contains(forbidden),
                "Core must not restore Runtime-owned PluginHost composition: {forbidden}"
            );
        }
        let runtime_provider_composition =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/provider_composition.rs"))
                .expect("read runtime provider composition");
        assert!(
            runtime_provider_composition.contains("pub async fn dispatch_provider_list_patch(")
        );
        assert!(runtime_provider_composition.contains("pub trait ProviderListPatchTarget"));
        assert!(runtime_provider_composition.contains("pub fn apply_provider_list_patch<T>("));
        assert!(runtime_provider_composition.contains("pub fn provider_descriptors_from_ids<I>("));
        assert!(runtime_provider_composition.contains("plugins.is_empty()"));
        assert!(
            runtime_provider_composition
                .contains("dispatch_provider_list(ProviderListInput { current })")
        );
        assert!(core_plugin_registry.contains("agena_runtime::dispatch_provider_list_patch"));
        assert!(
            core_plugin_registry
                .contains("agena_runtime::provider_descriptors_from_ids(registry.provider_ids())")
        );
        assert!(
            core_plugin_registry
                .contains("agena_runtime::apply_provider_list_patch(&mut registry, patch)")
        );
        assert!(
            !core_plugin_registry
                .contains("dispatch_provider_list(agena_plugin_host::ProviderListInput")
        );
        assert!(!core_plugin_registry.contains("for provider_id in patch.remove"));
        assert!(builders.contains(
            "pub(super) async fn build_runtime_provider_registry(\n    providers: &std::collections::BTreeMap"
        ));
        assert!(builders.contains(
            "pub(super) async fn build_model_catalog_services(\n    inputs: agena_runtime::ModelCatalogCompositionInputs"
        ));
        assert!(!builders.contains("fn collect_watch_paths("));
        let runtime_watch_paths =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/watch_paths.rs"))
                .expect("read runtime watch path composition");
        assert!(runtime_watch_paths.contains("pub fn runtime_watch_paths("));
        assert!(runtime_watch_paths.contains("PluginsConfig"));
        assert!(!builders.contains("ConfigResolution"));
    }

    #[test]
    fn provider_model_discovery_consumes_provider_map() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/config/adapter_models.rs"))
                .expect("read provider adapter model discovery");
        assert!(source.contains("list_provider_adapter_models_with_providers"));
        assert!(source.contains("providers: &BTreeMap<String, ResolvedProviderConfig>"));
        assert!(!source.contains("list_provider_adapter_models_with_config"));
        let snapshot =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/snapshot/mod.rs"))
                .expect("read runtime snapshot");
        assert!(snapshot.contains("pub(crate) fn provider_configs("));
        assert!(!snapshot.contains("fn provider_catalog_priorities("));
        assert!(!snapshot.contains("fn model_catalog_snapshot("));
        assert!(!snapshot.contains("fn provider_model("));
        assert!(!snapshot.contains("fn model_capabilities_for("));
        let builder =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/builder.rs"))
                .expect("read runtime builder catalog refresh");
        assert!(builder.contains(
            "agena_runtime::provider_model_catalog_priorities(snapshot.provider_configs())"
        ));
        assert!(snapshot.contains("pub(crate) fn plugin_storage("));
        assert!(snapshot.contains("pub(crate) fn plugin_secret_store("));
        assert!(snapshot.contains("pub(crate) fn config_value("));
        assert!(snapshot.contains("pub(crate) fn resolved_config_value("));
        assert!(snapshot.contains("agena_runtime::config_resolution_json_value("));
        assert!(snapshot.contains("agena_runtime::resolved_config_json_value("));
        assert!(!snapshot.contains("serde_json::to_value"));
        let runtime_resolved = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/config_values/resolved.rs"),
        )
        .expect("read Runtime configuration JSON projections");
        assert!(runtime_resolved.contains("pub fn config_resolution_json_value("));
        assert!(runtime_resolved.contains("pub fn resolved_config_json_value("));
        assert!(runtime_resolved.contains("pub fn applied_layer_descriptions("));
        assert!(snapshot.contains("self.resolution_meta.applied_layer_descriptions()"));
        assert!(snapshot.contains("pub(crate) fn tracing_config("));
        assert!(snapshot.contains("pub(crate) fn plugin_config("));
        assert!(snapshot.contains("pub(crate) fn default_agent("));
        assert!(snapshot.contains("pub(crate) fn default_provider("));
        assert!(snapshot.contains("pub(crate) fn ui_config("));
        assert!(snapshot.contains("pub(crate) fn config_path("));
        assert!(snapshot.contains("pub(crate) fn project_config_path("));
        assert!(snapshot.contains("pub(crate) fn applied_layer_descriptions("));
        let service =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog_service.rs"))
                .expect("read runtime model catalog service");
        assert!(service.contains("provider_priorities: Option<&ProviderModelPriorities>"));
        assert!(!service.contains("ResolvedProviderConfig"));
        assert!(!service.contains("Option<&ConfigResolution>"));
        let provider_priorities =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/provider_priorities.rs"))
                .expect("read Runtime provider catalog priorities");
        assert!(provider_priorities.contains("pub fn provider_model_catalog_priorities("));
        let core_registry =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/config/registry.rs"))
                .expect("read Core provider registry");
        assert!(!core_registry.contains("fn provider_model_catalog_priorities("));
        let selection = fs::read_to_string(
            workspace.join("apps/agena/src/backend/backend_provider/selection.rs"),
        )
        .expect("read provider selection backend");
        assert!(!selection.contains("config_resolution()"));
    }

    #[test]
    fn app_presentation_does_not_reopen_runtime_resolution() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in ["apps/agena/src", "apps/agena-studio-server/src"] {
            let source = collect_rust_sources(&workspace.join(relative))
                .expect("read app presentation sources")
                .join("\n");
            assert!(
                !source.contains("config_resolution()"),
                "app presentation must use snapshot projections: {relative}"
            );
        }
        let settings =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/settings.rs"))
                .expect("read API settings presentation source");
        assert!(!settings.contains("config_resolution()"));
        for relative in [
            "crates/agena-application/src/dispatch/mod.rs",
            "crates/agena-cli/src/cli/cli_runtime.rs",
            "crates/agena-api-server/src/rest/auth.rs",
            "crates/agena-api-server/src/rest/marketplace.rs",
        ] {
            let source = fs::read_to_string(workspace.join(relative))
                .expect("read migrated presentation source");
            assert!(!source.contains("config_resolution()"), "{relative}");
        }
        let app = fs::read_to_string(workspace.join("apps/agena/src/app.rs"))
            .expect("read app message boundary");
        assert!(!app.contains("agena::message::ExecutionStatus"));
    }

    #[test]
    fn terminal_startup_uses_the_single_cli_parser() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let core_lib = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
            .expect("read legacy core module exports");
        assert!(!core_lib.contains("pub mod cli") && !core_lib.contains("mod cli"));
        assert!(!workspace.join("crates/agena-runtime/src/cli.rs").exists());
        let entrypoint = fs::read_to_string(workspace.join("apps/agena/src/main.rs"))
            .expect("read terminal app entrypoint");
        assert!(
            entrypoint.contains("use agena_cli::") && entrypoint.contains("AgenaCli"),
            "terminal entrypoint must delegate parsing to agena-cli"
        );
        assert!(
            !entrypoint.contains("derive(Debug, Parser)")
                && !entrypoint.contains("derive(Debug, Clone, Parser)"),
            "terminal entrypoint must not define a second clap parser"
        );

        let cli = fs::read_to_string(workspace.join("crates/agena-cli/src/cli/mod.rs"))
            .expect("read terminal CLI parser");
        assert!(
            cli.contains("#[derive(Debug, Clone, Parser)]"),
            "agena-cli must own the terminal parser"
        );
        assert!(
            cli.contains("parser_routes_bare_invocation_to_tui_mode")
                && cli.contains("parser_keeps_subcommands_in_command_mode"),
            "agena-cli parser must have launch-mode smoke contracts"
        );
    }

    #[test]
    fn default_workspace_build_is_terminal_only() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest =
            fs::read_to_string(workspace.join("Cargo.toml")).expect("read workspace manifest");
        assert!(
            manifest.contains("default-members = [\"apps/agena\"]"),
            "default workspace build must stay focused on the terminal app"
        );
        assert!(
            !manifest.contains("default-members = [\"apps/agena\", \"apps/agena-studio-server\"]"),
            "Studio must not be part of the default build"
        );
    }

    #[test]
    fn aggregate_runtime_config_facades_are_deleted() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let snapshot =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/snapshot/mod.rs"))
                .expect("read runtime snapshot");
        let builder =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/builder.rs"))
                .expect("read runtime builder");
        assert!(!snapshot.contains("fn config_resolution("));
        assert!(!builder.contains("fn config_resolution("));
        assert!(snapshot.contains("RuntimeSnapshotState<Arc<ResolvedConfig>"));
        assert!(snapshot.contains("resolution_meta: ConfigResolutionMeta"));
        assert!(!snapshot.contains("ConfigResolution,"));
        let plugin_registry = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/config/registry/plugin_host.rs"),
        )
        .expect("read plugin registry adapters");
        assert!(!plugin_registry.contains("impl crate::config::ConfigResolution"));
        let provider_registry = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/config/registry/provider_registry.rs"),
        )
        .expect("read provider registry adapters");
        assert!(!provider_registry.contains("impl ResolvedConfig"));
        let runtime_mod =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/mod.rs"))
                .expect("read runtime module exports");
        assert!(!runtime_mod.contains("pub use event_bridge::spawn_event_bridge"));
        assert!(!runtime_mod.contains("pub mod host_client"));
        assert!(!runtime_mod.contains("pub use host_client::"));
        let host_client_mappers = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime/host_client/mappers.rs"),
        )
        .expect("read core host-client mappers");
        assert!(
            !host_client_mappers.contains("mod active_invocations"),
            "logical invocation reentrancy must remain runtime-owned"
        );
        let host_client = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime/host_client/mod.rs"),
        )
        .expect("read core host-client adapter");
        assert!(
            host_client.contains("agena_runtime::try_enter_invocation"),
            "core host-client adapter must consume the runtime invocation guard"
        );
        let runtime_lib = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
            .expect("read runtime exports");
        assert!(runtime_lib.contains("try_enter_invocation"));
        assert!(runtime_lib.contains("SessionExecutionControl"));
        assert!(
            runtime_lib.contains("RuntimeApplicationServices"),
            "runtime must export the application-facing builder result"
        );
        let runtime_application_services =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/application_services.rs"))
                .expect("read runtime application service bundle");
        for required in [
            "pub struct RuntimeApplicationServices",
            "pub provider_catalog: Arc<dyn ProviderCatalog>",
            "pub event_queries: Option<Arc<dyn RuntimeEventQueryService>>",
            "pub execution_commands: Option<Arc<dyn SessionExecutionCommandService>>",
        ] {
            assert!(
                runtime_application_services.contains(required),
                "runtime application builder result must retain `{required}`"
            );
        }
        let runtime_execution_control = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/session_execution_control.rs"),
        )
        .expect("read runtime session execution-control port");
        assert!(
            runtime_execution_control.contains(
                "fn snapshot_status(&self, workspace_root: &Path) -> Option<RuntimeSnapshotStatus>"
            ),
            "runtime execution control must expose a stable snapshot-status projection"
        );
        let session_manager = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/session/manager/sessions.rs"),
        )
        .expect("read core session execution adapter");
        assert!(
            session_manager
                .contains("impl agena_runtime::SessionExecutionControl for SessionManager"),
            "core session manager must implement the runtime execution-control port"
        );
        assert!(
            session_manager.contains("fn snapshot_status("),
            "core session manager must adapt snapshot state through the runtime control port"
        );
        assert!(
            session_manager.contains("agena_runtime::RuntimeSnapshotStatus { active, managed }"),
            "core session manager must project snapshot state instead of returning its registry"
        );
        let application_git =
            fs::read_to_string(workspace.join("crates/agena-application/src/service/git.rs"))
                .expect("read application git service");
        assert!(
            application_git.contains("Option<&dyn agena_runtime::SessionExecutionControl>"),
            "application git services must accept the runtime execution-control port"
        );
        for forbidden in [
            "agena::runtime::AgenaRuntime",
            "SessionManager",
            "tool_executor()",
            "snapshot_registry()",
            "list_active_snapshots",
            "list_managed_snapshots",
        ] {
            assert!(
                !application_git.contains(forbidden),
                "application git services must not traverse core session/runtime state through `{forbidden}`"
            );
        }
        for forbidden in [
            "pub use snapshot_managed::{ManagedSnapshot, list_managed_snapshots};",
            "pub use snapshot_registry::{",
        ] {
            assert!(
                !runtime_lib.contains(forbidden),
                "runtime must not re-export concrete snapshot state through `{forbidden}`"
            );
        }
        let api_git = fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/git.rs"))
            .expect("read API git endpoint");
        assert!(
            api_git.contains("state.git_status()")
                && api_git.contains("state.snapshot_status()")
                && api_git.contains("state.git_stage(request)")
                && !api_git.contains("session_execution_control()"),
            "API git endpoints must consume Application Git/snapshot use cases rather than extract Runtime control ports"
        );
        assert!(
            !api_git.contains("state.runtime()") && !api_git.contains("state.service().git_"),
            "API git endpoints must not bypass application Git use cases"
        );
        let runtime_catalog_service = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/model_catalog_runtime_service.rs"),
        )
        .expect("read runtime model-catalog service port");
        assert!(
            runtime_catalog_service.contains("pub trait ModelCatalogRuntimeService"),
            "runtime must own the stable catalog snapshot/refresh service port"
        );
        let core_runtime_builder =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/builder.rs"))
                .expect("read core runtime catalog adapter");
        assert!(
            core_runtime_builder
                .contains("impl agena_runtime::ModelCatalogRuntimeService for AgenaRuntime"),
            "core runtime must adapt catalog composition through the runtime service port"
        );
        let api_model_catalog =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/model_catalog.rs"))
                .expect("read API model-catalog endpoint");
        let application_catalog =
            fs::read_to_string(workspace.join("crates/agena-application/src/application.rs"))
                .expect("read Application model-catalog use cases");
        let api_state = fs::read_to_string(workspace.join("crates/agena-api-server/src/state.rs"))
            .expect("read API state runtime accessors");
        assert!(
            api_model_catalog.contains("state.list_model_catalog_with_origin(")
                && api_model_catalog.contains("state.lookup_model_catalog_models(")
                && api_model_catalog.contains("state.refresh_model_catalog()")
                && application_catalog.contains("pub fn list_model_catalog_with_origin(")
                && application_catalog.contains("pub fn refresh_model_catalog(")
                && !api_state.contains("fn model_catalog_runtime")
                && !application_catalog.contains("pub fn model_catalog_runtime"),
            "API model-catalog endpoints must consume Application list/lookup/refresh use cases rather than expose the Runtime catalog service"
        );
        for forbidden in [
            "state.runtime()",
            "state.model_catalog_runtime()",
            "ModelCatalogRuntimeService",
            "model_catalog_response()",
            "start_model_catalog_refresh(",
            "AgenaRuntime",
            "current_snapshot()",
        ] {
            assert!(
                !api_model_catalog.contains(forbidden),
                "API model-catalog endpoints must not reach core runtime state through `{forbidden}`"
            );
        }
        let sqlite_catalog_adapter = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/model_catalog_repository.rs"),
        )
        .expect("read SQLite model-catalog repository adapter");
        assert!(
            sqlite_catalog_adapter
                .contains("impl ModelCatalogRepository for SeaModelCatalogRepository"),
            "SQLite infrastructure crate must own the model-catalog storage adapter"
        );
        assert!(
            !sqlite_catalog_adapter.contains("agena_core")
                && !sqlite_catalog_adapter.contains("crate::db::entities"),
            "SQLite catalog adapter must not depend on core entities or the core crate"
        );
        let core_db_mod = fs::read_to_string(workspace.join("crates/agena-runtime/src/db/mod.rs"))
            .expect("read core database module");
        assert!(
            !core_db_mod.contains("sea_model_catalog_repository"),
            "core DB module must not retain the moved catalog cache adapter"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/db/sea_model_catalog_repository.rs")
                .exists(),
            "the old core catalog cache adapter must be deleted"
        );
        let snapshot_builders = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime/snapshot/builders.rs"),
        )
        .expect("read core snapshot builders");
        assert!(
            snapshot_builders.contains("ModelCatalogService::compose_default_optional(database)"),
            "core composition must delegate optional catalog storage composition to Runtime"
        );
        assert!(
            !snapshot_builders.contains("ModelCatalogService::compose_default("),
            "core composition must not bypass Runtime catalog startup policy"
        );
        assert!(
            !snapshot_builders.contains("SeaModelCatalogRepository"),
            "core snapshot composition must not construct the SQLite catalog adapter directly"
        );
        let runtime_model_selection = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/provider_model_selection.rs"),
        )
        .expect("read Runtime configured model selection projection");
        assert!(
            runtime_model_selection.contains("pub fn configured_local_models(")
                && runtime_model_selection.contains("pub fn configured_enabled_adapter_ids("),
            "Runtime must own configured local-model route/default selection policy"
        );
        let core_snapshot =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/snapshot/mod.rs"))
                .expect("read core snapshot model selection adapter");
        assert!(
            core_snapshot
                .contains("agena_runtime::configured_local_models(provider_id, configured)"),
            "core snapshot must consume Runtime configured local-model projection"
        );
        assert!(
            core_snapshot.contains("map(agena_runtime::configured_enabled_adapter_ids)"),
            "core snapshot must consume Runtime enabled-adapter projection"
        );
        assert!(
            !core_snapshot.contains("for route in configured.models.keys()"),
            "core snapshot must not reconstruct configured local-model route policy"
        );
        let core_model_catalog =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog/mod.rs"))
                .expect("read core model-catalog facade");
        assert!(
            !core_model_catalog.contains("fn canonical_model_catalog_id")
                && !core_model_catalog.contains("normalized_catalog_model_id")
                && !core_model_catalog.contains("pub(crate) use agena_runtime"),
            "core model-catalog module must not reintroduce the Runtime ID normalization policy"
        );
        let sqlite_projection_lookup = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/projection_lookup_repository.rs"),
        )
        .expect("read SQLite projection lookup adapter");
        assert!(
            sqlite_projection_lookup
                .contains("impl ProjectionLookupRepository for SeaProjectionLookupRepository"),
            "SQLite infrastructure crate must own the projected message/part lookup adapter"
        );
        assert!(
            !sqlite_projection_lookup.contains("crate::db::entities")
                && !sqlite_projection_lookup.contains("agena_core"),
            "SQLite projection lookup adapter must not depend on core entities or the core crate"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/db/sea_projection_lookup_repository.rs")
                .exists(),
            "the old core projection lookup adapter must be deleted"
        );
        let core_session_manager =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/manager/mod.rs"))
                .expect("read core session composition");
        assert!(
            core_session_manager.contains("agena_storage_sqlite::SeaProjectionLookupRepository"),
            "session composition must use the SQLite projection lookup adapter"
        );
        let sqlite_session_stats = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/session_stats_repository.rs"),
        )
        .expect("read SQLite session stats adapter");
        assert!(
            sqlite_session_stats
                .contains("impl SessionStatsRepository for SeaSessionStatsRepository"),
            "SQLite infrastructure crate must own the session statistics adapter"
        );
        assert!(
            sqlite_session_stats.contains("MESSAGE_CREATED_EVENT_KIND_TAGS"),
            "SQLite session statistics must consume stable domain message-event tags"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/db/sea_session_stats_repository.rs")
                .exists(),
            "the old core session statistics adapter must be deleted"
        );
        assert!(
            core_session_manager.contains("agena_storage_sqlite::SeaSessionStatsRepository"),
            "session composition must use the SQLite session statistics adapter"
        );
        let sqlite_workspace = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/workspace_repository.rs"),
        )
        .expect("read SQLite workspace adapter");
        assert!(
            sqlite_workspace.contains("impl WorkspaceRepository for SeaWorkspaceRepository"),
            "SQLite infrastructure crate must own the workspace repository adapter"
        );
        assert!(
            !sqlite_workspace.contains("crate::db::entities")
                && !sqlite_workspace.contains("agena_core"),
            "SQLite workspace adapter must not depend on core entities or the core crate"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/db/sea_workspace_repository.rs")
                .exists(),
            "the old core workspace adapter must be deleted"
        );
        assert!(
            core_session_manager.contains("agena_storage_sqlite::SeaWorkspaceRepository"),
            "session composition must use the SQLite workspace adapter"
        );
        let application_commands =
            fs::read_to_string(workspace.join("crates/agena-application/src/dispatch/commands.rs"))
                .expect("read application command dispatch");
        assert!(
            application_commands.contains("session_services\n                .execution_control\n                .cancel_active_execution"),
            "application cancellation must use the runtime execution-control port"
        );
        let application_execution =
            fs::read_to_string(workspace.join("crates/agena-application/src/service/execution.rs"))
                .expect("read application execution service");
        assert!(
            application_execution.contains("execution_control.list_scheduled_jobs()"),
            "application scheduler inspection must use the runtime execution-control port"
        );
        assert!(
            application_execution
                .contains("execution_control\n                    .selected_model(session_id)"),
            "application run-option resolution must use the runtime model-selection port"
        );
        let application_dispatch =
            fs::read_to_string(workspace.join("crates/agena-application/src/dispatch/mod.rs"))
                .expect("read application runtime-status dispatch");
        assert!(
            application_dispatch.contains("state.runtime_status().runtime_status().await"),
            "application runtime status must use the stable runtime-status port"
        );
        let application_queries =
            fs::read_to_string(workspace.join("crates/agena-application/src/dispatch/queries.rs"))
                .expect("read application query dispatch");
        assert!(
            !application_queries
                .contains("let manager = state.session_manager()?;\n    match query"),
            "application query dispatch must acquire the concrete session adapter only in branches that still require it"
        );
        assert!(
            application_queries.contains("let queries = state.session_query_service()?;")
                && !application_queries.contains("state.session_manager()"),
            "all transcript reads must use the session query port without materializing a concrete manager"
        );
        let application_messages =
            fs::read_to_string(workspace.join("crates/agena-application/src/service/messages.rs"))
                .expect("read application message service");
        assert!(
            application_messages.contains("if query.parts == PartLoadMode::None")
                && application_messages.contains("list_projected_message_headers(session_id)"),
            "application header-only message lists must use the runtime projected-message header port"
        );
        assert!(
            application_messages.contains("find_session_id_for_message(message_id)")
                && application_messages.contains("find_session_id_for_part(part_id)"),
            "message and part ownership must use runtime query ports instead of materializing a concrete manager"
        );
        assert!(
            application_queries
                .contains("let session_services = state.session_execution_services()?;")
                && application_queries.contains("session_services.execution_control.as_ref()")
                && application_queries.contains("session_services.queries.as_ref()"),
            "session-state dispatch must pass runtime control/query ports rather than a concrete manager"
        );
        for forbidden in [
            "state.runtime()",
            "current_snapshot()",
            "SessionExecutionControl::cache_stats",
        ] {
            assert!(
                !application_dispatch.contains(forbidden),
                "application runtime-status dispatch must not traverse concrete runtime state through `{forbidden}`"
            );
        }
        let runtime_status_service = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime_status_service.rs"),
        )
        .expect("read runtime status service port");
        assert!(
            runtime_status_service.contains("pub trait RuntimeStatusService"),
            "runtime must own the stable operational status service port"
        );
        assert!(
            runtime_status_service.contains("pub struct RuntimeStatusSnapshot"),
            "runtime status port must return a typed projection rather than an opaque payload"
        );
        assert!(
            core_runtime_builder
                .contains("impl agena_runtime::RuntimeStatusService for AgenaRuntime"),
            "core runtime must adapt concrete snapshots through the runtime status port"
        );
        let plugin_runtime_service = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/plugin_runtime_service.rs"),
        )
        .expect("read runtime plugin service port");
        assert!(
            plugin_runtime_service.contains("pub trait PluginRuntimeService"),
            "runtime must own the stable read-only plugin service port"
        );
        assert!(
            core_runtime_builder
                .contains("impl agena_runtime::PluginRuntimeService for AgenaRuntime"),
            "core runtime must adapt concrete plugin host state through the runtime plugin port"
        );
        let api_plugin_routes =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/plugins.rs"))
                .expect("read API plugin routes");
        for required in [
            "state.plugin_runtime().plugin_statuses()",
            "plugin_runtime.plugin_ui_catalog()",
            "plugin_runtime.tool_registry_events_since(",
            ".plugin_runtime()\n        .plugin_inspect",
            "plugin_runtime.plugin_logs(",
        ] {
            assert!(
                api_plugin_routes.contains(required),
                "API plugin read routes must use the runtime plugin port through `{required}`"
            );
        }
        let api_rest =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/mod.rs"))
                .expect("read API base routes");
        let application_handle =
            fs::read_to_string(workspace.join("crates/agena-application/src/application.rs"))
                .expect("read application handle");
        assert!(
            api_rest.contains("state.runtime_snapshot_summary().await")
                && !api_rest.contains("state.runtime_status()")
                && application_handle.contains("pub(crate) fn runtime_status(&self)")
                && !application_handle.contains("pub fn runtime_status(&self)"),
            "API health, readiness, and metrics must consume the Application runtime-summary projection"
        );
        let runtime_configuration_service = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime_configuration_service.rs"),
        )
        .expect("read runtime configuration service port");
        assert!(
            runtime_configuration_service.contains("pub trait RuntimeConfigurationService"),
            "runtime must own the stable configuration projection port"
        );
        assert!(
            core_runtime_builder
                .contains("impl agena_runtime::RuntimeConfigurationService for AgenaRuntime"),
            "core runtime must adapt its resolved configuration through the runtime configuration port"
        );
        let api_settings =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/settings.rs"))
                .expect("read API settings routes");
        assert!(
            api_settings.contains("state.config_json_sources()")
                && !api_settings.contains("runtime_configuration()"),
            "API settings routes must consume the Application configuration-source projection"
        );
        for forbidden in ["state.runtime()", "current_snapshot()"] {
            assert!(
                !api_settings.contains(forbidden),
                "API settings routes must not traverse concrete runtime state through `{forbidden}`"
            );
        }
        let runtime_authentication_service = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime_authentication_service.rs"),
        )
        .expect("read runtime authentication service port");
        assert!(
            runtime_authentication_service.contains("pub trait RuntimeAuthenticationService"),
            "runtime must own provider credential and login lifecycle operations"
        );
        assert!(
            core_runtime_builder
                .contains("impl agena_runtime::RuntimeAuthenticationService for AgenaRuntime"),
            "core runtime must adapt resolved provider authentication through the runtime authentication port"
        );
        let application =
            fs::read_to_string(workspace.join("crates/agena-application/src/application.rs"))
                .expect("read application authentication port composition");
        assert!(
            application.contains(
                "runtime_authentication: Arc<dyn agena_runtime::RuntimeAuthenticationService>"
            ),
            "application must inject the runtime authentication port"
        );
        let api_auth =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/auth.rs"))
                .expect("read API authentication routes");
        let application_auth =
            fs::read_to_string(workspace.join("crates/agena-application/src/application.rs"))
                .expect("read Application authentication use cases");
        for required in [
            "state.auth_providers()",
            "auth_provider_json_from_state(&state, provider_id.as_str())",
            ".auth_provider(provider_id)",
            ".set_auth_api_key(provider_id.as_str(), request.api_key)",
            ".remove_auth_provider(provider_id.as_str())",
            ".refresh_auth_provider(provider_id.as_str())",
            ".finish_auth_browser(",
            ".poll_auth_device(",
        ] {
            assert!(
                api_auth.contains(required),
                "API authentication routes must consume the migrated Application auth use case `{required}`"
            );
        }
        for required in [
            "pub fn auth_providers(",
            "pub fn auth_provider(",
            "pub async fn set_auth_api_key(",
            "pub async fn remove_auth_provider(",
            "pub async fn refresh_auth_provider(",
            "pub async fn finish_auth_browser(",
            "pub async fn poll_auth_device(",
            "async fn reload_after_authentication_change(",
            "pub enum AuthLoginKind",
        ] {
            assert!(application_auth.contains(required));
        }
        for forbidden in [
            "state.runtime()",
            "current_snapshot()",
            "AuthManager",
            "ProviderConfigCredentialStore",
            "ResolvedProviderConfig",
            "RuntimeAuthLoginKind",
            "RuntimeAuthProvider",
            "RuntimeAuthenticationError",
            "reload_runtime_from_config",
        ] {
            assert!(
                !api_auth.contains(forbidden),
                "API authentication routes must not retain concrete authentication traversal through `{forbidden}`"
            );
        }
        let api_state = fs::read_to_string(workspace.join("crates/agena-api-server/src/state.rs"))
            .expect("read API application composition state");
        assert!(
            !api_state.contains("runtime_authentication"),
            "API state must not expose RuntimeAuthenticationService after Application auth migration"
        );
        assert!(
            !application_auth.contains("pub fn runtime_authentication("),
            "Application must not re-expose RuntimeAuthenticationService after owning auth use cases"
        );
        let runtime_control_service = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime_control_service.rs"),
        )
        .expect("read runtime control service port");
        assert!(
            runtime_control_service.contains("pub trait RuntimeControlService"),
            "runtime must own the lifecycle and background-task control port"
        );
        assert!(
            core_runtime_builder
                .contains("impl agena_runtime::RuntimeControlService for AgenaRuntime"),
            "core runtime must adapt lifecycle controls through the runtime control port"
        );
        for required in [
            "state\n        .runtime_control()\n        .start_runtime_reload_task",
            ".runtime_control()\n            .background_tasks()",
            "state\n        .runtime_control()\n        .cancel_background_task",
            ".runtime_control()\n        .reload()",
        ] {
            assert!(
                api_rest.contains(required),
                "API runtime-control routes must use the runtime control port through `{required}`"
            );
        }
        let api_marketplace =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/marketplace.rs"))
                .expect("read API marketplace routes");
        assert!(
            api_marketplace.contains(".runtime_control()\n        .start_background_task("),
            "API marketplace work must register through the runtime control port"
        );
        assert!(
            api_marketplace.contains("state.config_path().map_err(ServerError::from)?")
                && !api_marketplace.contains("runtime_configuration()"),
            "API marketplace work must consume the Application configuration-path projection"
        );
        let application_handle =
            fs::read_to_string(workspace.join("crates/agena-application/src/application.rs"))
                .expect("read application composition handle");
        assert!(
            !application_handle.contains("\n    runtime: AgenaRuntime"),
            "application must not retain a concrete core runtime handle after port composition"
        );
        assert!(
            application_handle.contains("pub fn from_composed_runtime_services("),
            "application must consume the Runtime-owned composed capability bundle"
        );
        assert!(
            !application_handle.contains("use agena::{runtime::AgenaRuntime"),
            "application construction must not import the concrete runtime builder"
        );
        assert!(
            !application_handle.contains("SessionManager"),
            "application must retain session behavior only through runtime ports"
        );
        let api_state = fs::read_to_string(workspace.join("crates/agena-api-server/src/state.rs"))
            .expect("read API application composition state");
        assert!(
            api_state.contains("pub fn from_application(application: Application) -> Self"),
            "API state must consume the already-composed application handle"
        );
        assert!(
            !api_state.contains("Application::from_runtime_services("),
            "API state must not compose runtime services itself"
        );
        assert!(
            !api_state.contains("pub struct AppState {\n    runtime:"),
            "API state must not retain the concrete runtime after application composition"
        );
        let api_manifest = fs::read_to_string(workspace.join("crates/agena-api-server/Cargo.toml"))
            .expect("read API server manifest");
        assert!(
            !api_manifest.contains("[dependencies]\nagena = { workspace = true }"),
            "API server must not retain a normal Core dependency"
        );
        assert!(
            !api_manifest.contains("agena = { workspace = true }"),
            "API contract tests must use Runtime-owned fixtures after Core deletion"
        );
        let runtime_config_settings = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime_config_settings_service.rs"),
        )
        .expect("read runtime config-settings service");
        let domain_json_path =
            fs::read_to_string(workspace.join("crates/agena-domain/src/json_path.rs"))
                .expect("read Domain JSON path value module");
        assert!(
            runtime_config_settings.contains("pub trait RuntimeConfigSettingsService")
                && runtime_config_settings.contains("pub(crate) fn get_json_path(")
                && runtime_config_settings.contains("agena_domain::get_json_path"),
            "runtime must use the Domain JSON-path value while retaining the stable settings-editing port"
        );
        assert!(
            domain_json_path.contains("pub fn parse_json_path")
                && domain_json_path.contains("pub fn get_json_path")
                && domain_json_path.contains("pub fn format_json_path"),
            "Domain must own the schema-neutral JSON-path grammar and lookup values"
        );
        for relative in [
            "apps/agena/src/app.rs",
            "apps/agena/src/backend/backend_config.rs",
            "apps/agena/src/app/plugin_workbench/workbench_schema_util.rs",
            "crates/agena-api-server/src/rest/mod.rs",
        ] {
            let source = fs::read_to_string(workspace.join(relative))
                .expect("read terminal JSON-path consumer");
            assert!(
                !source.contains("agena::config::get_json_path"),
                "JSON-path consumer must not restore the old Core configuration projection: {relative}"
            );
            assert!(
                !source.contains("agena_runtime::get_json_path"),
                "JSON-path consumer must use Domain values rather than Runtime's old public helper: {relative}"
            );
        }
        let runtime_bootstrap =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/bootstrap_request.rs"))
                .expect("read Runtime bootstrap request");
        assert!(
            runtime_bootstrap.contains("pub struct RuntimeBootstrapRequest")
                && runtime_bootstrap.contains("config_override_expressions: Vec<String>")
                && runtime_bootstrap.contains("pub database_url: Option<String>")
                && runtime_bootstrap.contains("pub database_path: Option<PathBuf>")
                && !runtime_bootstrap.contains("DatabaseConnection")
                && !runtime_bootstrap.contains("pub database_connection"),
            "Runtime bootstrap input must own raw override and database URL/path intent without concrete database types"
        );
        let runtime_bootstrap_result =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/bootstrap_result.rs"))
                .expect("read Runtime bootstrap result");
        assert!(
            runtime_bootstrap_result.contains("pub struct RuntimeBootstrapResult")
                && runtime_bootstrap_result.contains("RuntimeApplicationServices")
                && runtime_bootstrap_result.contains("RuntimeBootstrapLifecycle")
                && runtime_bootstrap_result.contains("into_application_services")
                && runtime_bootstrap_result
                    .contains("pub(crate) async fn compose_runtime_bootstrap")
                && runtime_bootstrap_result.contains("RuntimeBootstrapComposition")
                && !runtime_bootstrap_result.contains("DatabaseConnection")
                && !runtime_bootstrap_result.contains("database_connection"),
            "Runtime bootstrap result must expose application/repository contracts without a concrete database return"
        );
        assert!(
            core_runtime_builder.contains("agena_runtime::compose_runtime_bootstrap(")
                && !core_runtime_builder.contains("RuntimeBootstrapResult::new("),
            "Core bootstrap must delegate request/result envelope composition to Runtime"
        );
        let runtime_environment =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/config_environment.rs"))
                .expect("read Runtime configuration environment boundary");
        assert!(
            runtime_environment.contains("pub trait ConfigEnvironment")
                && runtime_environment.contains("pub struct ProcessEnvironment"),
            "Runtime must own schema-neutral configuration environment access"
        );
        let runtime_output_format =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/output_format.rs"))
                .expect("read Runtime output-format contract");
        let core_provider_config = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/config_values/provider.rs"),
        )
        .expect("read Runtime provider configuration values");
        assert!(
            runtime_output_format.contains("pub enum OutputFormat")
                && runtime_output_format.contains("pub struct OutputFormatParseError")
                && !core_provider_config.contains("ConfigOutputFormat"),
            "Runtime must own the schema-neutral process output-format contract rather than Core configuration"
        );
        let core_config_loader =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/config/loader.rs"))
                .expect("read Core configuration loader");
        assert!(
            core_config_loader
                .contains("pub(crate) use agena_runtime::{ConfigEnvironment, ProcessEnvironment};")
                && !core_config_loader.contains("pub trait ConfigEnvironment"),
            "Core loader must consume Runtime configuration environment access without a public compatibility re-export"
        );
        let core_config_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/config/mod.rs"))
                .expect("read Core configuration module");
        for legacy_export in [
            "pub use error::ConfigError;",
            "pub use registry::build_provider_registry_from_configs;",
            "pub use adapter_models::{",
            "pub use credential_store::{",
            "pub use registry::ProviderAdapterModelsResult;",
            "pub use registry::list_provider_adapter_models;",
        ] {
            assert!(
                !core_config_module.contains(legacy_export),
                "Core configuration adapter must not publicly re-export `{legacy_export}`"
            );
        }
        assert!(
            core_config_module.contains("pub(crate) use agena_runtime::ConfigError;")
                && !core_config_module.contains("enum ConfigError"),
            "Runtime configuration adapters must reuse the Runtime-owned configuration error"
        );
        let runtime_config_paths =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/config_paths.rs"))
                .expect("read Runtime configuration path policy");
        for required in [
            "pub fn default_config_path(",
            "pub(crate) fn default_workspace_root()",
            "pub(crate) fn project_config_path(",
        ] {
            assert!(
                runtime_config_paths.contains(required),
                "Runtime must own configuration path policy `{required}`"
            );
        }
        for forbidden in [
            "fn default_config_path(",
            "fn default_workspace_root()",
            "fn project_config_path(",
        ] {
            assert!(
                !core_config_loader.contains(forbidden),
                "Core loader must not retain configuration path policy `{forbidden}`"
            );
        }
        for relative in [
            "crates/agena-cli/src/cli/cli_runtime.rs",
            "apps/agena-studio-server/src/app.rs",
            "tools/agena-e2e/src/bin/dsv4f_tool_api_suite.rs",
            "tools/agena-e2e/src/bin/dsv4f_tool_api_probe.rs",
        ] {
            let source = fs::read_to_string(workspace.join(relative))
                .expect("read Runtime bootstrap consumer");
            assert!(
                source.contains("RuntimeBootstrapRequest"),
                "bootstrap consumer must use the Runtime-owned request: {relative}"
            );
            assert!(
                !source.contains("AgenaRuntimeConfig"),
                "bootstrap consumer must not construct Core AgenaRuntimeConfig: {relative}"
            );
        }
        let terminal_main_source = fs::read_to_string(workspace.join("apps/agena/src/main.rs"))
            .expect("read terminal app-server bootstrap source");
        let terminal_tui_source = fs::read_to_string(workspace.join("apps/agena/src/lib.rs"))
            .expect("read terminal TUI bootstrap source");
        let cli_runtime_source =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_runtime.rs"))
                .expect("read CLI bootstrap source");
        let studio_source =
            fs::read_to_string(workspace.join("apps/agena-studio-server/src/app.rs"))
                .expect("read Studio bootstrap source");
        for (relative, source) in [
            ("apps/agena/src/main.rs", terminal_main_source.as_str()),
            ("apps/agena/src/lib.rs", terminal_tui_source.as_str()),
            (
                "crates/agena-cli/src/cli/cli_runtime.rs",
                cli_runtime_source.as_str(),
            ),
            (
                "apps/agena-studio-server/src/app.rs",
                studio_source.as_str(),
            ),
        ] {
            assert!(
                source.contains("database_path")
                    && !source.contains("StorageConfig")
                    && !source.contains("ensure_parent("),
                "bootstrap consumer must pass database intent to Runtime instead of composing storage: {relative}"
            );
        }
        for relative in [
            "apps/agena/Cargo.toml",
            "apps/agena-studio-server/Cargo.toml",
            "crates/agena-api-server/Cargo.toml",
        ] {
            let manifest = fs::read_to_string(workspace.join(relative))
                .expect("read upper-layer bootstrap manifest");
            assert!(
                !manifest.contains("agena-storage"),
                "upper-layer bootstrap consumer must not retain a storage dependency after Runtime takes URL/path resolution: {relative}"
            );
        }
        let runtime_bootstrap_request =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/bootstrap_request.rs"))
                .expect("read Runtime bootstrap request adapter");
        let runtime_bootstrap_result =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/bootstrap_result.rs"))
                .expect("read Runtime bootstrap composition adapter");
        assert!(
            !core_runtime_builder.contains("from_bootstrap_request(")
                && core_runtime_builder.contains("pub async fn bootstrap_application_services(")
                && core_runtime_builder.contains("pub(crate) async fn new(")
                && core_runtime_builder
                    .contains("agena_runtime::compose_runtime_bootstrap(request,")
                && !core_runtime_builder.contains("into_composition_config()")
                && !core_runtime_builder.contains("expression.parse()")
                && runtime_bootstrap_request.contains("load_config_request_from_bootstrap")
                && runtime_bootstrap_request.contains("into_composition_config")
                && runtime_bootstrap_result
                    .contains("pub(crate) async fn compose_runtime_bootstrap")
                && runtime_bootstrap_result.contains("request.into_composition_config()?"),
            "Core must delegate bootstrap input normalization and raw override parsing to Runtime while returning Runtime results to port consumers"
        );
        let core_lib = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
            .expect("read Core crate root");
        let storage_contract =
            fs::read_to_string(workspace.join("crates/agena-storage/src/lib.rs"))
                .expect("read storage contract");
        assert!(
            storage_contract.contains("pub struct StorageConfig")
                && storage_contract.contains("pub enum StorageConfigError")
                && !core_lib.contains("pub mod storage"),
            "database URL/path bootstrap policy must be storage-owned rather than a Core facade"
        );
        for relative in [
            "apps/agena/src/main.rs",
            "apps/agena/src/lib.rs",
            "apps/agena-studio-server/src/app.rs",
            "crates/agena-cli/src/cli/mod.rs",
            "crates/agena-cli/src/cli/cli_runtime.rs",
            "crates/agena-cli/src/cli/cli_render.rs",
        ] {
            let source =
                fs::read_to_string(workspace.join(relative)).expect("read StorageConfig consumer");
            assert!(
                !source.contains("agena::storage::StorageConfig")
                    && !source.contains("    storage::StorageConfig,"),
                "StorageConfig consumer must import the storage contract rather than Core: {relative}"
            );
        }
        let dsv4f_probe =
            fs::read_to_string(workspace.join("tools/agena-e2e/src/bin/dsv4f_tool_api_probe.rs"))
                .expect("read DSV4F Tool API probe");
        assert!(
            dsv4f_probe.contains("bootstrap_application_services")
                && dsv4f_probe.contains(".set_session_allowed_tools(")
                && dsv4f_probe.contains(".set_session_permission(")
                && dsv4f_probe.contains(".pending_interactive_requests(")
                && dsv4f_probe.contains(".list_projected_messages(")
                && !dsv4f_probe.contains("SessionManager")
                && !dsv4f_probe.contains("get_session("),
            "DSV4F Tool API probe must use Runtime session ports and projections rather than Core session aggregates"
        );
        let dsv4f_suite =
            fs::read_to_string(workspace.join("tools/agena-e2e/src/bin/dsv4f_tool_api_suite.rs"))
                .expect("read DSV4F Tool API suite");
        assert!(
            dsv4f_suite.contains("bootstrap_application_services")
                && dsv4f_suite.contains("available_tool_api_definitions()")
                && !dsv4f_suite.contains("available_tool_api_bindings()")
                && !dsv4f_suite.contains("ToolApiBinding::definition")
                && !dsv4f_suite.contains("SessionManager")
                && !dsv4f_suite.contains("get_session("),
            "DSV4F Tool API suite must inspect provider Tool API definitions and session state through Runtime rather than Core executor/session aggregates"
        );
        let cli_render =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_render.rs"))
                .expect("read CLI renderer");
        let cli_runtime =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_runtime.rs"))
                .expect("read CLI runtime helpers");
        let cli_module = fs::read_to_string(workspace.join("crates/agena-cli/src/cli/mod.rs"))
            .expect("read CLI module");
        assert!(
            cli_module.contains("pub enum CliError")
                && cli_module.contains("type AppError = CliError")
                && !cli_module.contains("error::AppError"),
            "CLI must own its presentation/process error boundary instead of importing Core AppError"
        );
        assert!(
            cli_runtime.contains("map_err(|error| AppError::Internal(error.to_string()))"),
            "the temporary Core bootstrap seam must map into the CLI-owned error boundary"
        );
        let runtime_provider_client_versions = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/provider_client_versions.rs"),
        )
        .expect("read Runtime provider client-version refresh");
        assert!(
            runtime_provider_client_versions
                .contains("pub async fn fetch_latest_provider_client_versions(")
                && runtime_provider_client_versions.contains("valid_client_version")
                && runtime_provider_client_versions.contains("set_provider_client_versions")
                && runtime_provider_client_versions.contains("claude_user_web_fetch_user_agent"),
            "Runtime must own provider client-version refresh, active state, and HTTP identities"
        );
        assert!(
            runtime_control_service.contains("async fn fetch_provider_client_versions(")
                && core_runtime_builder.contains("async fn fetch_provider_client_versions(")
                && application_handle.contains(".fetch_provider_client_versions()")
                && !application_handle.contains("agena_runtime::fetch_latest_provider_client_versions()")
                && !runtime_lib.contains("pub use provider_client_versions::{\n    ProviderClientVersionFetchError, fetch_latest_provider_client_versions,\n};"),
            "Application must reach provider client-version HTTP refresh through RuntimeControlService rather than a public Runtime free function"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/provider/runtime.rs")
                .exists(),
            "Core must not retain provider client-version state or HTTP identities"
        );
        let terminal_backend_workspace =
            fs::read_to_string(workspace.join("apps/agena/src/backend/backend_workspace.rs"))
                .expect("read terminal provider workspace adapter");
        assert!(
            terminal_backend_workspace
                .contains("self.application\n            .refresh_provider_client_versions()")
                && application_handle.contains("pub async fn refresh_provider_client_versions(")
                && !terminal_backend_workspace.contains("fetch_latest_provider_client_versions")
                && !terminal_backend_workspace
                    .contains("agena::provider::fetch_latest_provider_client_versions"),
            "terminal provider refresh must use the Application command rather than Core or Runtime fetch choreography"
        );
        let cli_command_runner =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_run.rs"))
                .expect("read CLI command runner for lifecycle retention");
        assert!(
            cli_render.contains("with_session_runtime_services(|services| async move")
                && cli_render.contains("execute_runtime_tool(&input, -1, -1)")
                && !cli_render.contains("ToolExecutor::new(")
                && !cli_render.contains("Agent::new(")
                && !cli_render.contains("ToolPayloadInput::ApplyPatch"),
            "CLI apply must invoke the Runtime tool service rather than composing a Core executor"
        );
        assert!(
            cli_runtime.contains("bootstrap_application_services")
                && cli_runtime.contains("operation(runtime.application_services()).await")
                && cli_runtime.contains("runtime.shutdown();")
                && cli_runtime.contains("Result<agena_runtime::RuntimeBootstrapResult, AppError>")
                && !cli_runtime.contains("from_bootstrap_request(")
                && !cli_runtime.contains("into_application_services()"),
            "CLI runtime helpers must return/retain the Runtime bootstrap result rather than a concrete runtime handle"
        );
        assert!(
            cli_module.contains("pub overrides: Vec<String>")
                && cli_module.contains("pub config_override_expressions: Vec<String>")
                && cli_module.contains("OutputFormat")
                && !cli_module.contains("ConfigOverrideArgument")
                && !cli_module.contains("LoadConfigRequest")
                && !cli_module.contains("ConfigOutputFormat")
                && cli_runtime.contains("config_override_expressions: self.overrides.clone()")
                && !cli_runtime.contains("LoadConfigRequest"),
            "CLI launch intent must retain raw --set expressions and must not expose Core configuration input types"
        );
        let cli_permissions =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_permissions.rs"))
                .expect("read CLI permission composition");
        assert!(
            !cli_permissions.contains("runtime.database_connection()")
                && cli_permissions.contains("Application::from_composed_runtime_services")
                && !cli_permissions.contains("SeaWorkspaceRepository")
                && cli_render.contains("application_from_runtime(&runtime)")
                && !cli_render.contains("permission_database(")
                && !cli_render.contains("connect_database(")
                && !cli_render.contains("resolved_tracing_config"),
            "CLI must consume Runtime-composed repository ports rather than locally resolving concrete adapters"
        );
        assert!(
            cli_module.contains("runtime: agena_runtime::RuntimeBootstrapResult")
                && cli_module.contains("fn shutdown(&self)")
                && cli_runtime.contains("runtime,")
                && cli_command_runner.contains("let lifecycle = backend.clone();")
                && cli_command_runner.contains("lifecycle.shutdown();"),
            "CLI MCP server must retain and explicitly shut down its Runtime bootstrap result for the server lifetime"
        );
        let terminal_main = fs::read_to_string(workspace.join("apps/agena/src/main.rs"))
            .expect("read terminal application entrypoint for lifecycle retention");
        let terminal_tui = fs::read_to_string(workspace.join("apps/agena/src/lib.rs"))
            .expect("read terminal TUI composition");
        assert!(
            terminal_main.contains("runtime: runtime.clone()")
                && terminal_main.contains("runtime.shutdown();")
                && terminal_main.contains("runtime: agena_runtime::RuntimeBootstrapResult")
                && !terminal_main.contains("runtime.into_application_services()"),
            "App Server must retain and explicitly shut down its Runtime bootstrap result rather than consume only service handles"
        );
        assert!(
            terminal_main.contains("agena_runtime::runtime_env_filter")
                && terminal_tui.contains("agena_runtime::runtime_env_filter")
                && !terminal_tui.contains("runtime.database_connection()")
                && !terminal_tui.contains("agena_runtime::connect_database")
                && !terminal_main.contains("agena::tracing::")
                && !terminal_tui.contains("agena::tracing::"),
            "terminal process tracing must consume Runtime projection without reopening a database connection"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/tracing.rs")
                .exists(),
            "Core must not retain a process tracing helper facade"
        );
        let runtime_configuration = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime_configuration_service.rs"),
        )
        .expect("read Runtime configuration projection");
        let application_runtime_dto =
            fs::read_to_string(workspace.join("crates/agena-application/src/dto/runtime.rs"))
                .expect("read application runtime DTOs");
        let terminal_backend = fs::read_to_string(workspace.join("apps/agena/src/backend.rs"))
            .expect("read terminal backend composition");
        let terminal_backend_plugins =
            fs::read_to_string(workspace.join("apps/agena/src/backend/backend_plugins.rs"))
                .expect("read terminal backend plugin projection");
        let terminal_backend_session =
            fs::read_to_string(workspace.join("apps/agena/src/backend/backend_session.rs"))
                .expect("read terminal backend session projection");
        let terminal_provider_settings = fs::read_to_string(
            workspace.join("apps/agena/src/backend/backend_provider/settings.rs"),
        )
        .expect("read terminal provider settings projection");
        let terminal_provider_selection = fs::read_to_string(
            workspace.join("apps/agena/src/backend/backend_provider/selection.rs"),
        )
        .expect("read terminal provider selection projection");
        let terminal_provider_draft_validation = fs::read_to_string(
            workspace.join("apps/agena/src/backend/backend_drafts/provider_draft_validation.rs"),
        )
        .expect("read terminal provider draft listing projection");
        let cli_run = fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_run.rs"))
            .expect("read CLI command runner");
        assert!(
            runtime_configuration.contains("pub configuration_document: serde_json::Value")
                && runtime_configuration.contains("pub project_config_path: PathBuf")
                && runtime_configuration.contains("pub applied_layers: Vec<String>")
                && core_runtime_builder
                    .contains("configuration_document = snapshot\n            .config_value()")
                && cli_run.contains("configuration.configuration_document")
                && cli_render.contains("configuration_document\n                .get(\"meta\")"),
            "Runtime configuration projection must retain the complete read-only resolution document for CLI config and diagnostics commands"
        );
        assert!(
            application_runtime_dto.contains("pub struct ConfigJsonSources")
                && application_handle.contains("pub fn config_json_sources(&self) -> Result<ConfigJsonSources, ApplicationError>")
                && application_handle.contains("pub fn config_agent_names(&self) -> HashSet<String>")
                && application_handle.contains("pub fn config_path(&self) -> Result<PathBuf, ApplicationError>")
                && !application_handle.contains("fn runtime_configuration(")
                && terminal_backend_workspace.contains("self.application.config_agent_names()")
                && terminal_backend_workspace.contains("self.application\n            .config_path()")
                && terminal_backend_workspace.contains("self.application\n            .config_json_sources()")
                && terminal_backend_workspace.contains("set_project_file_setting")
                && terminal_backend_workspace.contains("delete_project_file_setting")
                && terminal_backend_workspace.contains("runtime_control()")
                && terminal_backend_workspace.contains("refresh_provider_client_versions()")
                && application_handle.contains("pub async fn refresh_provider_client_versions(")
                && !terminal_backend_workspace.contains("runtime_configuration()")
                && !terminal_backend_workspace.contains("augment_effective_config_json")
                && !terminal_backend.contains("pub struct ConfigJsonSources")
                && !terminal_backend_workspace.contains("fetch_latest_provider_client_versions")
                && !terminal_backend_workspace.contains(
                    "let snapshot = self.runtime.current_snapshot();\n        let config_path"
                ),
            "Terminal configuration reads must consume the Application source projection; writes use Runtime settings/control ports without concrete fetch/persistence choreography or a snapshot"
        );
        assert!(
            !terminal_backend.contains("runtime: AgenaRuntime")
                && terminal_backend.contains("application: Application")
                && terminal_backend.contains("workspace_root: PathBuf")
                && terminal_backend_workspace
                    .contains("services: agena_runtime::RuntimeApplicationServices")
                && terminal_backend_workspace
                    .contains("Application::from_composed_runtime_services(services)")
                && !terminal_backend_workspace.contains("DatabaseConnection")
                && !terminal_backend_workspace.contains("SeaWorkspaceRepository"),
            "Terminal Backend must consume Runtime-composed application repository ports"
        );
        assert!(
            terminal_backend_plugins.contains("plugin_runtime()")
                && terminal_backend_plugins.contains("permission_tool_catalog()")
                && terminal_backend_plugins.contains("resolve_plugin_tool")
                && terminal_backend_plugins.contains("invoke_plugin_command")
                && terminal_backend_plugins.contains("render_session_tool_output")
                && !terminal_backend_plugins.contains("current_snapshot()")
                && !terminal_backend_plugins.contains("plugin_manager()")
                && !terminal_backend_plugins.contains("session_manager()"),
            "Terminal plugin presentation and invocation must consume PluginRuntimeService rather than traverse a Core snapshot"
        );
        assert!(
            terminal_backend_workspace.contains("provider_catalog()")
                && terminal_backend_workspace.contains("provider_summary_resource_from_catalog")
                && terminal_backend_workspace.contains("ProviderCatalogEntry"),
            "Terminal provider listing must project ProviderCatalog entries rather than read Core provider registry/configuration types"
        );
        assert!(
            terminal_backend_workspace.contains("agent_statuses()")
                && terminal_backend_workspace.contains("agent_profile(name)")
                && terminal_backend_workspace.contains("default_agent_name()")
                && terminal_backend_workspace.contains("runtime_snapshot_summary().await")
                && terminal_backend_workspace.contains("TuiPreferencesResource")
                && terminal_backend_workspace.contains("tui_preferences()")
                && terminal_backend_workspace.contains("fn ui_configuration(")
                && !terminal_backend_workspace.contains("RuntimeUiConfiguration")
                && !terminal_backend_workspace.contains(".runtime_status()")
                && !terminal_backend_workspace.contains("current_snapshot()"),
            "Terminal workspace/agent/status/UI-preference presentation must consume Application projections rather than Runtime values or a Core snapshot"
        );
        assert!(
            terminal_tui.contains("fn tui_config_from_preferences(")
                && terminal_tui.contains("TuiColorSchemeResource")
                && terminal_tui.contains("TuiGraphicsModeResource")
                && !terminal_tui.contains("RuntimeUiConfiguration")
                && !terminal_tui.contains("RuntimeTuiColorScheme")
                && !terminal_tui.contains("RuntimeTuiGraphicsMode")
                && !terminal_tui.contains("tui_config_from_persistent")
                && !terminal_tui.contains("TuiColorSchemeConfig")
                && !terminal_tui.contains("TuiGraphicsModeConfig"),
            "Terminal startup and reload must map Application UI preferences rather than Runtime or Core UI values"
        );
        let terminal_app_sources = collect_rust_sources(&workspace.join("apps/agena/src"))
            .expect("collect terminal application sources for agent-projection audit");
        assert!(
            terminal_backend_workspace.contains("RuntimeAgentResource")
                && terminal_backend_workspace.contains("RuntimeAgentProfileResource")
                && !terminal_backend_workspace.contains("agena_runtime::RuntimeAgentStatus")
                && !terminal_backend_workspace.contains("agena_runtime::RuntimeAgentProfile")
                && terminal_app_sources
                    .iter()
                    .all(|source| !source.contains("agena::agents::"))
                && terminal_app_sources
                    .iter()
                    .all(|source| !source.contains("agena::agent::PermissionConfig"))
                && terminal_app_sources.iter().all(|source| {
                    !source.contains("agena::message::AttachmentItem")
                        && !source.contains("agena::message::AttachmentKind")
                        && !source.contains("agena::message::AttachmentSource")
                }),
            "Terminal agent and attachment presentation must consume Runtime/plugin-SDK projections rather than reconstruct Core values"
        );
        assert!(
            terminal_backend_session.contains("session_query_service()")
                && terminal_backend_session.contains("ApiCommand::CancelRun")
                && !terminal_backend_session.contains("session_execution_control()")
                && terminal_backend_session.contains("set_session_agent")
                && terminal_backend_session.contains("set_session_permission")
                && terminal_backend_session.contains("execution_context(session_id)")
                && terminal_backend_session.contains("selected_permission")
                && terminal_backend_session.contains("event_query_service()")
                && terminal_backend_session.contains("RuntimeReverseEventRange")
                && terminal_backend_session.contains("RuntimeEventRange")
                && terminal_backend_session.contains("list_timeline_events_before")
                && terminal_backend_session.contains("event_stream_service()")
                && terminal_backend_session
                    .contains("is_descendant_session(descendant_id, session_id)")
                && !terminal_backend_session.contains("manager.event_bus()")
                && !terminal_backend_session.contains("session_manager()")
                && terminal_backend_session.contains("session_user_message_part_from_wire")
                && terminal_backend_session.contains(".steer_input(session_id, parts)")
                && terminal_backend_plugins.contains("session_execution_services()")
                && !terminal_backend_plugins.contains("session_execution_control()")
                && terminal_backend_plugins.contains("tool_execution")
                && terminal_backend_plugins.contains("execute_snapshot_command")
                && terminal_backend_plugins.contains("invoke_session_plugin_command")
                && !terminal_backend_plugins.contains("let executor = manager.tool_executor();"),
            "Terminal session usage/control, agent selection, steer input, snapshot inspection/commands, and plugin command/tool execution must consume application commands and Runtime session ports without a concrete manager"
        );
        assert!(
            terminal_provider_settings.contains("runtime_config_settings()")
                && terminal_provider_settings.contains("read_file_settings")
                && terminal_provider_settings.contains("patch_file_settings")
                && terminal_provider_settings.contains("runtime_control()")
                && !terminal_provider_settings.contains("current_snapshot().config_path()"),
            "Terminal provider settings file selection, writes, and reload must consume Runtime configuration/settings/control ports"
        );
        assert!(
            terminal_provider_selection.contains("provider_catalog()")
                && terminal_provider_selection.contains("configured_routing")
                && terminal_provider_selection.contains("configured_editor")
                && terminal_provider_selection.contains("configured_local_models")
                && terminal_provider_selection.contains("list_saved_adapter_models")
                && terminal_provider_selection.contains("list_draft_adapter_models")
                && terminal_provider_selection.contains("list_model_catalog(query, offset, limit)")
                && terminal_provider_selection.contains("lookup_model_catalog_models(model_ids)")
                && terminal_provider_selection.contains("request_model_catalog_refresh()")
                && application_handle.contains("pub fn list_model_catalog(")
                && application_handle.contains("pub fn lookup_model_catalog_models(")
                && application_handle.contains("pub fn request_model_catalog_refresh(")
                && terminal_provider_selection.contains("default_model()")
                && terminal_provider_selection.contains("model_execution_options(model)")
                && terminal_provider_selection.contains("runtime_config_settings()")
                && terminal_provider_selection.contains("read_file_settings")
                && terminal_provider_draft_validation.contains("build_listing_request")
                && terminal_provider_draft_validation
                    .contains("DraftProviderAdapterModelsRequest::BedrockSigv4")
                && !terminal_provider_draft_validation.contains("ProviderAdapterModelsTarget")
                && !terminal_provider_settings.contains("list_provider_adapter_models_with_target")
                && !terminal_provider_selection.contains("model_catalog_runtime()")
                && !terminal_provider_selection.contains("start_model_catalog_refresh")
                && !terminal_provider_selection.contains(
                    "let resolved = snapshot\n            .provider_configs()\n            .get(provider_id)"
                ),
            "Provider discovery and model-catalog presentation must use ProviderCatalog/Application use cases rather than Runtime catalog values, Core targets, or a snapshot in the terminal backend"
        );
        let cli_auth_helpers =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_auth_helpers.rs"))
                .expect("read CLI authentication helpers");
        let runtime_oauth_callback =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/oauth_callback.rs"))
                .expect("read Runtime OAuth callback capability");
        assert!(
            cli_render.contains("with_application(")
                && cli_render.contains("application\n                        .auth_providers()")
                && cli_run.contains("start_auth_browser(")
                && cli_run.contains("complete_auth_browser_callback(")
                && cli_run.contains("start_auth_device(")
                && cli_run.contains("poll_auth_device(")
                && cli_run.contains("remove_auth_provider(")
                && !cli_render.contains("services.authentication")
                && !cli_run.contains("services.authentication")
                && !cli_auth_helpers.contains("ResolvedProviderConfig")
                && !cli_auth_helpers.contains("ProviderOAuthTarget")
                && !cli_auth_helpers.contains("ProviderDeviceAuthTarget")
                && !cli_auth_helpers.contains("RuntimeAuth")
                && !cli_run.contains("AuthManager::new(")
                && !cli_run.contains("ProviderConfigCredentialStore"),
            "CLI authentication must consume Application auth use cases rather than Runtime authentication or Core credential adapters"
        );
        assert!(
            runtime_oauth_callback.contains("pub fn wait_for_oauth_callback")
                && runtime_oauth_callback.contains("TcpListener")
                && runtime_oauth_callback.contains("error_description")
                && runtime_oauth_callback.contains("request_id")
                && runtime_oauth_callback.contains("escape_html(error)")
                && runtime_authentication_service.contains("fn wait_auth_browser_callback(")
                && core_runtime_builder.contains("fn wait_auth_browser_callback(")
                && application_auth.contains("pub async fn complete_auth_browser_callback(")
                && !cli_auth_helpers.contains("wait_auth_browser_callback")
                && !cli_run.contains("wait_auth_browser_callback")
                && !cli_auth_helpers.contains("agena_runtime::wait_for_oauth_callback")
                && !runtime_lib.contains("pub use oauth_callback::{\n    RuntimeOAuthCallbackError, parse_oauth_callback_url, wait_for_oauth_callback,")
                && !workspace
                    .join("crates/agena-runtime/src/provider/auth/oauth/callback.rs")
                    .exists(),
            "Runtime must exclusively own the diagnostic-preserving, HTML-escaped local OAuth callback listener"
        );
        let dsv4f_suite_support = fs::read_to_string(
            workspace.join("tools/agena-e2e/src/bin/dsv4f_tool_api_suite_support/mod.rs"),
        )
        .expect("read DSV4F Tool API suite support");
        assert!(
            dsv4f_suite_support.contains("RuntimeLiveEventSubscription")
                && dsv4f_suite_support.contains("subscribe_events(")
                && !dsv4f_suite_support.contains("event_bus()")
                && !dsv4f_suite_support.contains("bus::SubscriptionItem")
                && dsv4f_suite_support.contains("SuiteTranscript")
                && dsv4f_suite_support.contains("SessionProjectedOperationPart")
                && dsv4f_suite_support.contains("list_projected_messages(session_id, true)")
                && !dsv4f_suite_support.contains("session::Session")
                && !dsv4f_suite_support.contains("message::OperationPart")
                && !dsv4f_suite_support.contains("PartContent::Operation"),
            "DSV4F streaming and operation/history assertions must consume Runtime subscriptions and projected transcript values rather than Core event/session/message aggregates"
        );
        assert!(
            dsv4f_suite_support.contains("commands\n                .submit_user_message")
                && dsv4f_suite_support.contains("pending_interactive_requests(session_id)")
                && dsv4f_suite_support.contains(
                    "self.execution_commands\n                            .reply_user_input"
                )
                && dsv4f_suite_support.contains(
                    "self.execution_commands\n                            .reply_permission"
                ),
            "DSV4F model turns must use Runtime session commands and pending-request queries rather than Core session-manager execution"
        );
        assert!(
            dsv4f_suite_support.contains("self.execution_commands")
                && dsv4f_suite_support.contains(".set_session_allowed_tools(")
                && dsv4f_suite_support.contains(".set_session_permission(")
                && !dsv4f_suite_support.contains("self.manager\n            .create_session"),
            "DSV4F suite session setup must use Runtime execution commands rather than Core session-manager mutation"
        );
        let dsv4f_integration = fs::read_to_string(
            workspace.join("tools/agena-e2e/src/bin/dsv4f_tool_api_suite_cases/integration.rs"),
        )
        .expect("read DSV4F Tool API integration cases");
        assert!(
            dsv4f_integration.contains("session_presentation(child_id)")
                && dsv4f_integration.contains("active_execution(child_id)")
                && dsv4f_integration.contains("list_projected_messages(child_id, true)")
                && !dsv4f_integration.contains("manager.get_session(child_id)")
                && !dsv4f_integration.contains("manager.is_run_active(child_id)"),
            "DSV4F tasks.run child assertions must use Runtime session projections and execution control rather than Core session-manager reads"
        );
        let studio_main =
            fs::read_to_string(workspace.join("apps/agena-studio-server/src/main.rs"))
                .expect("read Studio process bootstrap");
        assert!(
            !studio_main.contains("agena::config")
                && !studio_main.contains("ConfigLoader")
                && !studio_main.contains("ConfigOverrideArgument"),
            "Studio process bootstrap must retain raw Runtime override input rather than resolve Core configuration before composition"
        );
        let studio_bootstrap =
            fs::read_to_string(workspace.join("apps/agena-studio-server/src/app.rs"))
                .expect("read Studio application bootstrap");
        assert!(
            !studio_bootstrap.contains(".database_connection()")
                && studio_bootstrap.contains("runtime.application_services()")
                && studio_bootstrap.contains("runtime.shutdown()")
                && !studio_bootstrap.contains("ConfigLoader::default()")
                && !studio_bootstrap.contains("connect_database("),
            "Studio must consume only Runtime application capabilities, not a database connection"
        );
        let api_settings =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/settings.rs"))
                .expect("read API settings routes");
        assert!(
            api_settings.contains(".runtime_config_settings()")
                && !api_settings.contains("agena::config"),
            "API settings routes must use the runtime settings port rather than Core configuration editing"
        );
        let api_server_sources =
            collect_rust_sources(&workspace.join("crates/agena-api-server/src"))
                .expect("collect API server sources for runtime facade scan");
        for source in api_server_sources {
            assert!(
                !source.contains("state.runtime()"),
                "API server must not reintroduce direct concrete runtime traversal"
            );
        }
        assert!(
            plugin_runtime_service.contains("async fn plugin_rpc("),
            "runtime plugin port must own authenticated plugin callback dispatch"
        );
        assert!(
            core_runtime_builder.contains("agena_runtime::dispatch_plugin_rpc("),
            "core runtime must adapt plugin callback dispatch through the runtime plugin port"
        );
        assert!(
            api_rest.contains(".plugin_runtime()\n        .plugin_rpc("),
            "API plugin RPC route must use the runtime plugin port"
        );
        for forbidden in [
            "state.runtime()",
            "current_snapshot()",
            "plugin_rpc_response(",
        ] {
            assert!(
                !api_rest.contains(forbidden),
                "API base routes must not retain concrete runtime/plugin callback traversal through `{forbidden}`"
            );
        }
        for required in [
            "fn resolve_studio_action(",
            "fn resolve_plugin_tool(",
            "async fn invoke_plugin_command(",
        ] {
            assert!(
                plugin_runtime_service.contains(required),
                "runtime plugin port must expose plugin UI operation `{required}`"
            );
        }
        let application_session =
            fs::read_to_string(workspace.join("crates/agena-application/src/application.rs"))
                .expect("read application session service bundle");
        assert!(
            application_session.contains(
                "pub tool_execution: Arc<dyn agena_runtime::SessionToolExecutionService>"
            ),
            "application session service bundle must expose the stable session tool-execution port"
        );
        assert!(
            application_session.contains(
                "pub plugin_commands: Arc<dyn agena_runtime::SessionPluginCommandService>"
            ),
            "application session service bundle must expose the session plugin-command port"
        );
        let session_plugin_command = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/session_plugin_command.rs"),
        )
        .expect("read runtime session plugin-command port");
        assert!(
            session_plugin_command.contains("pub trait SessionPluginCommandService"),
            "runtime must own session-scoped plugin-command authorization and invocation"
        );
        let core_session_manager =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/manager/mod.rs"))
                .expect("read core session manager plugin-command adapter");
        assert!(
            core_session_manager
                .contains("impl agena_runtime::SessionPluginCommandService for SessionManager"),
            "core session manager must adapt session plugin-command authorization"
        );
        assert!(
            api_plugin_routes
                .contains(".resolve_studio_action(plugin_id.as_str(), action_id.as_str())"),
            "API plugin UI actions must resolve through the runtime plugin port"
        );
        assert!(
            api_plugin_routes.contains(".resolve_plugin_tool(plugin_id, tool_name)"),
            "API plugin UI tool invocation must resolve through the runtime plugin port"
        );
        assert!(
            api_plugin_routes.contains(".tool_execution\n        .execute_session_tool"),
            "API plugin UI tool invocation must execute through the stable session tool port"
        );
        for forbidden in ["state.runtime()", "current_snapshot()"] {
            assert!(
                !api_plugin_routes.contains(forbidden),
                "API plugin routes must not retain concrete runtime traversal through `{forbidden}`"
            );
        }
        for forbidden in [
            "state.session_manager()",
            "tool_executor()",
            "resolve_tool_permission_check",
        ] {
            assert!(
                !api_plugin_routes.contains(forbidden),
                "API plugin routes must not retain concrete session permission traversal through `{forbidden}`"
            );
        }
        let api_session_routes =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/sessions.rs"))
                .expect("read API session routes");
        for required in [
            "async fn session_execution_json_from_id(",
            ".commands\n        .submit_user_message",
            ".commands\n        .continue_session",
            ".commands\n        .compact_session",
            ".commands\n        .fork_session",
            ".commands\n        .reply_permission",
            ".commands\n        .reply_user_input",
            ".commands\n        .rewind_session",
            ".commands\n        .import_session_jsonl",
            ".queries\n        .list_session_tree",
            ".queries\n        .export_session_jsonl",
        ] {
            assert!(
                api_session_routes.contains(required),
                "API session commands and queries must use runtime session ports through `{required}`"
            );
        }
        assert!(
            !api_session_routes.contains("session_execution_json_result"),
            "API session routes must not restore the concrete-session response helper"
        );
        let application_execution =
            fs::read_to_string(workspace.join("crates/agena-application/src/service/execution.rs"))
                .expect("read application event query service");
        assert!(
            application_execution.contains("events: &dyn agena_runtime::RuntimeEventQueryService"),
            "application after-sequence event queries must consume the runtime event-query port"
        );
        assert!(
            application_execution.contains("EventScope::Session { session_id }"),
            "application after-sequence event queries must scope storage reads to the requested session"
        );
        assert!(
            api_session_routes.contains("let events = state.application().event_query_service()?;"),
            "API session event backfill must obtain the runtime event-query port"
        );
        let storage_event_store =
            fs::read_to_string(workspace.join("crates/agena-storage-sqlite/src/event_store.rs"))
                .expect("read SQLite event store");
        assert!(
            storage_event_store.contains("async fn range_before("),
            "SQLite event store must implement the reverse cursor event range"
        );
        assert!(
            storage_event_store.contains("ORDER BY seq_global DESC LIMIT ?"),
            "SQLite reverse event range must fetch newest-first before application projection"
        );
        let application_sessions =
            fs::read_to_string(workspace.join("crates/agena-application/src/service/sessions.rs"))
                .expect("read application session event page service");
        assert!(
            application_sessions.contains("list_events_before("),
            "application cursor event pages must query the runtime reverse range"
        );
        assert!(
            application_sessions.contains("RuntimeReverseEventRange"),
            "application cursor event pages must use the runtime reverse-range contract"
        );
        let runtime_session_queries =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session_query_service.rs"))
                .expect("read runtime session query service");
        assert!(
            runtime_session_queries.contains("async fn is_descendant_session("),
            "runtime session query port must own persisted lineage checks for event streaming"
        );
        assert!(
            runtime_session_queries.contains("pub struct SessionProjectedMessageHeader"),
            "runtime session query port must own a stable projected-message header read model"
        );
        assert!(
            runtime_session_queries.contains("async fn list_projected_message_headers("),
            "runtime session query port must expose projected-message header reads"
        );
        assert!(
            runtime_session_queries.contains("async fn find_session_id_for_message("),
            "runtime session query port must resolve message ownership without exposing the concrete manager"
        );
        let core_session_history = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/session/manager/history.rs"),
        )
        .expect("read core session manager query adapter");
        assert!(
            core_session_history.contains("async fn is_descendant_session(\n        &self,"),
            "core session manager must adapt lineage checks through the session query port"
        );
        assert!(
            core_session_history.contains("async fn list_projected_message_headers("),
            "core session manager must adapt projected-message header reads through the session query port"
        );
        assert!(
            core_session_history.contains("async fn find_session_id_for_message("),
            "core session manager must adapt message ownership reads through the session query port"
        );
        assert!(
            api_session_routes.contains("let stream_service = state.event_stream_service()?;"),
            "API session SSE must subscribe through the runtime event-stream port"
        );
        assert!(
            api_session_routes.contains(".is_descendant_session(descendant_id, session_id)"),
            "API session SSE must query lineage through the runtime session port"
        );
        for forbidden in [
            "state.session_manager()",
            "SessionManager",
            "is_descendant_session(
    manager",
        ] {
            assert!(
                !api_session_routes.contains(forbidden),
                "API session routes must not retain concrete manager traversal through `{forbidden}`"
            );
        }
        let api_message_routes =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/messages.rs"))
                .expect("read API message routes");
        assert!(
            api_message_routes.contains("Query::ListMessages"),
            "API message list route must dispatch through the application query boundary"
        );
        assert!(
            api_message_routes.contains("Query::GetMessage"),
            "API message get route must dispatch through the application query boundary"
        );
        for required in ["Query::ListMessageParts", "Query::GetMessagePart"] {
            assert!(
                api_message_routes.contains(required),
                "API message part routes must dispatch through the application query boundary via `{required}`"
            );
        }
        assert!(
            !api_message_routes.contains("state.session_manager()"),
            "API message routes must not obtain a concrete session manager"
        );
        let api_queries = fs::read_to_string(workspace.join("crates/agena-api/src/queries.rs"))
            .expect("read API query contract");
        for required in [
            "ListMessageParts(ListMessagePartsParams)",
            "GetMessagePart(GetMessagePartParams)",
            "MessageParts(Vec<crate::message_part::MessagePartResource>)",
            "MessagePart(crate::message_part::MessagePartResource)",
        ] {
            assert!(
                api_queries.contains(required),
                "API query contract must own complete message part query value `{required}`"
            );
        }
        assert!(
            api_rest.contains("session_services\n            .queries\n            .usage_stats"),
            "API usage statistics must use the stable session query port"
        );
        let api_server_sources =
            collect_rust_sources(&workspace.join("crates/agena-api-server/src"))
                .expect("collect API server sources");
        for source in api_server_sources {
            assert!(
                !source.contains("state.session_manager()"),
                "API server must not obtain a concrete session manager"
            );
        }
        assert!(runtime_lib.contains("SessionQueryService"));
        assert!(runtime_lib.contains("SessionExecutionContext"));
        assert!(runtime_lib.contains("SessionExecutionCommandService"));
        assert!(runtime_lib.contains("SessionToolExecutionService"));
        let session_manager_source =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/manager/mod.rs"))
                .expect("read core session tool execution adapter");
        assert!(
            session_manager_source
                .contains("impl agena_runtime::SessionToolExecutionService for SessionManager"),
            "core session manager must implement the runtime session-tool port"
        );
        assert!(
            session_manager_source
                .contains("impl agena_runtime::SessionExecutionCommandService for SessionManager"),
            "core session manager must implement the runtime execution-command port"
        );
        let session_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/mod.rs"))
                .expect("read Runtime session module boundary");
        let tool_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/tool/mod.rs"))
                .expect("read Runtime tool result boundary");
        assert!(
            session_manager_source.contains("pub(crate) struct AuthorizedToolInvocation")
                && session_manager_source.contains("pub(crate) enum ToolInvocationAuthorization")
                && session_manager_source.contains("pub(crate) struct SessionSubtaskRequest")
                && session_manager_source.contains("pub(crate) struct SessionManager")
                && session_manager_source.contains("pub(crate) async fn authorize_session_tool_invocation")
                && session_manager_source.contains("pub(crate) async fn execute_host_invoked_tool")
                && session_module.contains("pub(crate) use manager::{SessionManager, SessionSubtaskRequest}")
                && session_module.contains("pub(crate) mod cost")
                && !session_module.contains("pub mod cost")
                && session_module.contains("pub(crate) use model::{")
                && session_module.contains("pub(crate) use processor::SessionProcessor")
                && session_module.contains("pub(crate) use history::ProjectedMessageHeader")
                && !session_module.contains("pub use model::{")
                && !session_module.contains("pub use processor::SessionProcessor")
                && !session_module.contains("pub use history::ProjectedMessageHeader")
                && fs::read_to_string(workspace.join("crates/agena-runtime/src/session/cost.rs"))
                    .expect("read Runtime session-cost adapter")
                    .contains("pub(crate) fn summarize")
                && tool_module.contains("pub(crate) use result::{ToolExecutionView, ToolInvocationExecution, ToolPayloadExecution}")
                && tool_module.contains("pub(crate) use payload::{ToolPayloadInput, ToolPayloadOutput}")
                && tool_module.contains("pub(crate) use truncation::ToolOutputTruncator")
                && tool_module.contains("pub(crate) use builtin_tools::BuiltinToolSet")
                && tool_module.contains("pub(crate) use self::tool_registry::*"),
            "detailed executor/session state, payloads, tool-set assembly, and authorization capabilities must remain Runtime-private behind SessionToolExecutionService"
        );
        let provider_module =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/mod.rs"))
                .expect("read Runtime provider credential boundary");
        assert!(
            provider_module.contains("pub(crate) use credential::{")
                && provider_module.contains("ManagedCredential")
                && provider_module.contains("parse_sap_ai_core_service_key")
                && provider_module.contains("should_retry_credential"),
            "managed provider credentials and their concrete parsing/retry helpers must remain Runtime-private"
        );
        let provider_core =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/core.rs"))
                .expect("read Runtime provider composition trait");
        assert!(
            provider_module.contains("pub(crate) use core::ModelRuntime")
                && provider_module
                    .contains("pub(crate) use crate::model_catalog::catalog_decoration_source")
                && provider_module.contains("pub(crate) mod auth")
                && !provider_module.contains("pub use core::ModelRuntime")
                && !provider_module
                    .contains("pub use crate::model_catalog::catalog_decoration_source")
                && !provider_module.contains("pub mod auth")
                && provider_core.contains("pub(crate) trait ModelRuntime"),
            "provider runtime composition, catalog-decoration, and credential/OAuth adapters must remain Runtime-private"
        );
        let runtime_builder =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/builder.rs"))
                .expect("read Runtime session-manager access boundary");
        let runtime_snapshot =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/snapshot/mod.rs"))
                .expect("read Runtime snapshot session-manager boundary");
        assert!(
            runtime_builder
                .contains("pub(crate) fn session_manager(&self) -> Option<Arc<SessionManager>>")
                && runtime_snapshot.contains(
                    "pub(crate) fn session_manager(&self) -> Option<Arc<SessionManager>>"
                ),
            "Runtime concrete session-manager accessors must remain crate-private"
        );
        let api_server_lib =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/lib.rs"))
                .expect("read API-server websocket contract fixture");
        assert!(
            api_server_lib.contains("RuntimeEventPublishRequest::PluginEvent")
                && !api_server_lib.contains("runtime\n            .session_manager()")
                && !api_server_lib.contains(".event_bus()"),
            "API-server websocket fixtures must publish through RuntimeEventPublishService, not a concrete session manager"
        );
        for (relative_path, label) in [
            (
                "apps/agena/src/backend/backend_plugins.rs",
                "terminal plugin backend",
            ),
            (
                "crates/agena-api-server/src/rest/plugins.rs",
                "API plugin endpoint",
            ),
        ] {
            let source = fs::read_to_string(workspace.join(relative_path))
                .unwrap_or_else(|_| panic!("read {label}"));
            assert!(
                source.contains("execute_session_tool"),
                "{label} must execute session tools through the runtime port"
            );
            assert!(
                !source.contains("ToolInvocationAuthorization"),
                "{label} must not expose the core session authorization enum"
            );
        }
        let session_history = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/session/manager/history.rs"),
        )
        .expect("read core session query adapter");
        assert!(
            session_history.contains("impl agena_runtime::SessionQueryService for SessionManager"),
            "core session manager must implement the runtime query port"
        );
        assert!(
            session_history.contains("async fn execution_context("),
            "core session manager must project execution state through the runtime query port"
        );
        assert!(
            session_history.contains("async fn list_session_summaries("),
            "core session manager must expose summary listing through the runtime query port"
        );
        assert!(
            session_history.contains("async fn session_presentation("),
            "core session manager must project stable session detail through the runtime query port"
        );
        let cli_render =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_render.rs"))
                .expect("read CLI session rendering");
        for required in [
            "let queries = services.session_queries.ok_or_else(session_storage_error)?;",
            ".export_session_jsonl(args.session_id)",
            ".list_session_tree(args.root_id)",
            ".import_session_jsonl(&bundle)",
            ".session_presentation(outcome.session_id)",
            ".session_cost_summary(session_id)",
            ".usage_stats(query)",
            ".continue_session(SessionExecutionRequest::new(session_id, options))",
            ".fork_session(SessionForkRequest {",
            ".create_session(SessionCreateRequest {",
            ".submit_user_message(SessionUserMessageRequest::new(",
            ".list_projected_messages(session.id, true)",
            ".reply_permission(SessionPermissionReplyRequest::new(",
        ] {
            assert!(
                cli_render.contains(required),
                "CLI session list/tree/export must use the runtime query port: {required}"
            );
        }
        for relative in [
            "crates/agena-cli/src/cli/cli_session_helpers.rs",
            "crates/agena-cli/src/cli/cli_auth_helpers.rs",
            "crates/agena-cli/src/cli/cli_runtime_helpers.rs",
        ] {
            let source = fs::read_to_string(workspace.join(relative))
                .expect("read CLI session query helper");
            assert!(
                source.contains("agena_runtime::SessionQueryService"),
                "CLI query helper must accept the runtime query port: {relative}"
            );
            assert!(
                !source.contains("agena::session::SessionManager"),
                "CLI query helper must not name the concrete session manager: {relative}"
            );
        }
        let cli_helpers =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_runtime_helpers.rs"))
                .expect("read CLI runtime helpers");
        assert!(
            !cli_helpers.contains("fn session_detail(session: &Session"),
            "CLI session details must be projected from Runtime rather than Core Session"
        );
        assert!(
            cli_helpers.contains(".selected_model(session_id)"),
            "CLI continue option resolution must obtain inherited models through Runtime control"
        );
        let cli_mod = fs::read_to_string(workspace.join("crates/agena-cli/src/cli/mod.rs"))
            .expect("read CLI MCP backend");
        assert!(
            !cli_mod.contains("session_manager: Option<Arc<SessionManager>>"),
            "CLI MCP backend must not retain a concrete session manager"
        );
        assert!(
            cli_mod
                .contains("session_queries: Option<Arc<dyn agena_runtime::SessionQueryService>>")
                && cli_mod.contains(
                    "event_publisher: Option<Arc<dyn agena_runtime::RuntimeEventPublishService>>"
                ),
            "CLI MCP backend must retain only Runtime session query/event ports"
        );
        assert!(
            cli_mod.contains("RuntimeEventPublishRequest::PluginEvent"),
            "CLI MCP audit events must cross the Runtime event-publish port"
        );
        let runtime_event_publish =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/event_publish_service.rs"))
                .expect("read Runtime event-publish contract");
        assert!(
            runtime_event_publish.contains("PluginEvent {")
                && runtime_event_publish.contains("plugin_id: agena_plugin_host::PluginKey"),
            "Runtime must own the typed plugin-event publication request"
        );
        let core_event_publish = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/session/manager/history.rs"),
        )
        .expect("read Core event-publish adapter");
        assert!(
            core_event_publish.contains("RuntimeEventPublishRequest::PluginEvent")
                && core_event_publish.contains("EventKind::PluginEvent"),
            "Core must adapt Runtime plugin-event publication to its concrete event bus"
        );
        assert!(
            application_commands.contains("state.session_execution_services()?"),
            "application commands must obtain runtime session ports from Application"
        );
        assert!(
            !application_commands.contains("SessionManager"),
            "application command dispatch must not name the core session manager"
        );
        for forbidden in [
            "manager.submit_user_message(",
            "manager.continue_session(",
            "manager.compact_session(",
            "manager.rewind_session(",
            "manager.fork_session(",
            "manager.import_session_jsonl(",
            "manager.reply_permission(",
            "manager.reply_user_input(",
            "manager.update_session_selection(",
        ] {
            assert!(
                !application_commands.contains(forbidden),
                "application commands must use the runtime execution-command port instead of `{forbidden}`"
            );
        }
        let application_session =
            fs::read_to_string(workspace.join("crates/agena-application/src/session.rs"))
                .expect("read application session adapters");
        assert!(
            application_session.contains("&dyn agena_runtime::SessionExecutionControl"),
            "session execution projection helper must accept the runtime control port"
        );
        assert!(
            application_session.contains("&dyn agena_runtime::SessionQueryService"),
            "session execution projection helper must accept the runtime query port"
        );
        assert!(
            application_session.contains("SessionUserMessagePart"),
            "application user-message projection must use the runtime content value"
        );
        assert!(
            !application_session.contains("agena::message::PartContent"),
            "application user-message projection must not import the core PartContent aggregate"
        );
        assert!(
            !application_session.contains("SessionManager"),
            "session execution projection helper must not expose SessionManager in its API"
        );
        assert!(
            application_execution
                .contains("session_queries\n            .latest_event_seq(session_id)"),
            "application execution projection must query the latest event sequence through the runtime port"
        );
        assert!(
            application_execution.contains("session_queries\n        .session_usage(session_id)"),
            "application execution projection must query usage through the runtime port"
        );
        assert!(
            application_execution
                .contains("session_queries\n            .execution_context(session_id)"),
            "application execution projection must query stable execution context through the runtime port"
        );
        assert!(
            !application_execution.contains("session: &Session"),
            "application execution projection must not accept the core Session aggregate"
        );
        let domain_pending = fs::read_to_string(
            workspace.join("crates/agena-domain/src/pending_interactive_request.rs"),
        )
        .expect("read domain pending-interactive-request value");
        assert!(domain_pending.contains("pub enum PendingInteractiveRequest"));
        let core_activity =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/message/part/activity.rs"))
                .expect("read core request-part activity projection");
        assert!(
            !core_activity.contains("pub enum PendingInteractiveRequest"),
            "pending interactive request must remain domain-owned"
        );
        let core_message =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/message/mod.rs"))
                .expect("read core message exports");
        assert!(
            !core_message.contains("PendingInteractiveRequest,"),
            "core message facade must not re-export the domain pending request"
        );
        assert!(
            application_execution
                .contains("session_queries\n        .pending_interactive_requests(session_id)"),
            "application pending-request projection must use the runtime session query port"
        );
        assert!(
            session_history.contains("PendingInteractiveRequestContext"),
            "core session query adapter must project pending requests into the domain context"
        );
        assert!(
            session_history.contains("async fn session_cost_summary("),
            "core session query adapter must provide domain cost summaries"
        );
        assert!(
            session_history.contains("async fn usage_stats("),
            "core session query adapter must provide domain usage statistics"
        );
        let cli_render =
            fs::read_to_string(workspace.join("crates/agena-cli/src/cli/cli_render.rs"))
                .expect("read CLI renderer");
        assert!(
            cli_render.contains(".session_cost_summary(session_id)"),
            "CLI cost rendering must use the runtime session query port"
        );
        assert!(
            !cli_render.contains("agena::session::cost::summarize"),
            "CLI cost rendering must not aggregate core message history directly"
        );
        assert!(
            cli_render.contains(".usage_stats(query)"),
            "CLI usage rendering must use the runtime session query port"
        );
        let domain_path_access =
            fs::read_to_string(workspace.join("crates/agena-domain/src/path_access.rs"))
                .expect("read domain path-access values");
        assert!(domain_path_access.contains("pub struct PathAccessModes"));
        let core_agent =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/agent/mod.rs"))
                .expect("read core agent permission adapter");
        assert!(
            !core_agent.contains("pub struct PathAccessModes"),
            "path access modes must remain domain-owned"
        );
        assert!(core_agent.contains("PathAccessModes"));
        assert!(domain_path_access.contains("pub enum PathAccessRuleConfig"));
        assert!(
            !core_agent.contains("pub enum PathAccessRuleConfig"),
            "path access rule shape must remain domain-owned"
        );
        assert!(core_agent.contains("fn path_access_rule_to_modes("));
        let domain_network_permission =
            fs::read_to_string(workspace.join("crates/agena-domain/src/network_permission.rs"))
                .expect("read domain network-permission values");
        assert!(domain_network_permission.contains("pub struct NetworkPermissionConfig"));
        assert!(
            !core_agent.contains("pub struct NetworkPermissionConfig"),
            "network permission configuration must remain domain-owned"
        );
        assert!(core_agent.contains("fn apply_network_permission_config("));
        let domain_tool_permission =
            fs::read_to_string(workspace.join("crates/agena-domain/src/tool_permission.rs"))
                .expect("read domain tool-permission values");
        assert!(domain_tool_permission.contains("pub enum ToolPermissionRules"));
        assert!(
            !core_agent.contains("pub enum ToolPermissionRules"),
            "tool permission rule shape must remain domain-owned"
        );
        assert!(core_agent.contains("fn apply_tool_permission_rules("));
        let domain_tool_permission_config =
            fs::read_to_string(workspace.join("crates/agena-domain/src/tool_permission_config.rs"))
                .expect("read domain tool-permission configuration");
        assert!(
            domain_tool_permission_config.contains("pub struct ToolPermissionConfig"),
            "tool permission configuration must remain domain-owned"
        );
        assert!(
            !core_agent.contains("pub struct ToolPermissionConfig"),
            "core must not redefine tool permission configuration"
        );
        assert!(
            core_agent.contains("fn apply_tool_permission_config("),
            "core must retain tool permission policy compilation"
        );
        let domain_path_permission =
            fs::read_to_string(workspace.join("crates/agena-domain/src/path_permission.rs"))
                .expect("read domain path-permission values");
        assert!(domain_path_permission.contains("pub struct PathPermissionConfig"));
        assert!(
            !core_agent.contains("pub struct PathPermissionConfig"),
            "path permission configuration must remain domain-owned"
        );
        assert!(core_agent.contains("fn apply_path_permission_config("));
        let domain_permission_config =
            fs::read_to_string(workspace.join("crates/agena-domain/src/permission_config.rs"))
                .expect("read domain aggregate permission configuration");
        assert!(
            domain_permission_config.contains("pub struct PermissionConfig"),
            "aggregate permission configuration must remain domain-owned"
        );
        assert!(
            !core_agent.contains("pub struct PermissionConfig"),
            "core must not redefine aggregate permission configuration"
        );
        for compiler in [
            "pub fn apply_to_permission_policy(",
            "pub fn apply_to_tool_permission_policy(",
            "pub fn apply_to_network_permission_policy(",
        ] {
            assert!(
                core_agent.contains(compiler),
                "core must retain concrete permission-policy compilation: {compiler}"
            );
        }
        let resolved = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/config_values/resolved.rs"),
        )
        .expect("read Runtime resolved config values");
        assert!(!resolved.contains("fn plugin_storage("));
        assert!(!resolved.contains("fn plugin_secret_store("));
        assert!(!resolved.contains("impl ResolvedConfig"));
        assert!(!resolved.contains("impl ConfigResolution {"));
        let provider_mod =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/mod.rs"))
                .expect("read provider module exports");
        for export in [
            "pub use agena_domain::{",
            "pub use amazon_bedrock::AmazonBedrockAdapter",
            "pub use anthropic::{AnthropicAdapter",
            "pub use gemini::{GeminiAdapter",
            "pub use gitlab::GitlabProvider",
            "pub use multi_adapter::MultiAdapterProvider",
            "pub use ollama::OllamaAdapter",
            "pub use registry::ProviderRegistry",
        ] {
            assert!(
                !provider_mod.contains(export),
                "legacy provider export: {export}"
            );
        }
        let message_mod =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/message/mod.rs"))
                .expect("read message module exports");
        assert!(!message_mod.contains("ExecutionStatus"));
        assert!(!message_mod.contains("PartKind"));
        assert!(!message_mod.contains("MessageSource"));
        let provider_contract =
            fs::read_to_string(workspace.join("crates/agena-provider/src/lib.rs"))
                .expect("read provider tool policy contract");
        assert!(provider_contract.contains("pub enum AgenaToolMode"));
        assert!(provider_contract.contains("pub struct AgenaToolsConfig"));
        assert!(provider_contract.contains("pub use network_config::{"));
        let provider_network_config =
            fs::read_to_string(workspace.join("crates/agena-provider/src/network_config.rs"))
                .expect("read provider network configuration contract");
        assert!(provider_network_config.contains("pub struct ProviderNetworkConfig"));
        assert!(provider_contract.contains("pub use route_config::{"));
        let provider_route_config =
            fs::read_to_string(workspace.join("crates/agena-provider/src/route_config.rs"))
                .expect("read provider route configuration contract");
        assert!(provider_route_config.contains("pub struct ProviderProtocolPathsConfig"));
        assert!(provider_route_config.contains("pub enum ProviderModelDiscoveryConfig"));
        assert!(provider_contract.contains("pub use secret_config::{"));
        let provider_secret_config =
            fs::read_to_string(workspace.join("crates/agena-provider/src/secret_config.rs"))
                .expect("read provider secret configuration contract");
        assert!(provider_secret_config.contains("pub enum ProviderSecretSourceConfig"));
        assert!(provider_secret_config.contains("pub enum ProviderGitlabApiAccessConfig"));
        assert!(provider_contract.contains("pub use bedrock_auth::BedrockSigv4AuthConfig;"));
        assert!(provider_contract.contains("pub use credential_config::{"));
        let provider_credential_config =
            fs::read_to_string(workspace.join("crates/agena-provider/src/credential_config.rs"))
                .expect("read provider credential configuration contract");
        for definition in [
            "pub struct ProviderInlineCredentialAuthConfig",
            "pub struct ProviderHttpCredentialAuthConfig",
            "pub struct ProviderSapAiCoreCredentialAuthConfig",
            "pub struct ProviderGitlabCredentialAuthConfig",
            "pub enum ProviderCredentialAuthConfig",
        ] {
            assert!(
                provider_credential_config.contains(definition),
                "provider must own credential-auth configuration: {definition}"
            );
        }
        let provider_bedrock_auth =
            fs::read_to_string(workspace.join("crates/agena-provider/src/bedrock_auth.rs"))
                .expect("read provider Bedrock auth contract");
        assert!(provider_bedrock_auth.contains("pub struct BedrockSigv4AuthConfig"));
        assert!(
            provider_contract
                .contains("pub use configured_model_config::ResolvedProviderModelConfig;")
        );
        let config_provider = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/config_values/provider.rs"),
        )
        .expect("read Runtime provider config types");
        assert!(!config_provider.contains("pub use agena_provider::AgenaToolMode;"));
        assert!(!config_provider.contains("pub enum AgenaToolMode"));
        assert!(!config_provider.contains("pub use agena_provider::AgenaToolsConfig;"));
        assert!(!config_provider.contains("pub struct AgenaToolsConfig"));
        assert!(!config_provider.contains("pub struct ProviderNetworkConfig"));
        assert!(!config_provider.contains("pub struct ProviderProtocolPathsConfig"));
        assert!(!config_provider.contains("pub enum ProviderModelDiscoveryConfig"));
        assert!(!config_provider.contains("pub enum ProviderSecretSourceConfig"));
        assert!(!config_provider.contains("pub enum ProviderGitlabApiAccessConfig"));
        assert!(!config_provider.contains("pub struct BedrockSigv4AuthConfig"));
        assert!(!config_provider.contains("pub struct ProviderInlineCredentialAuthConfig"));
        assert!(!config_provider.contains("pub struct ProviderHttpCredentialAuthConfig"));
        assert!(!config_provider.contains("pub struct ProviderSapAiCoreCredentialAuthConfig"));
        assert!(!config_provider.contains("pub struct ProviderGitlabCredentialAuthConfig"));
        assert!(!config_provider.contains("pub enum ProviderCredentialAuthConfig"));
        assert!(!config_provider.contains("pub struct ResolvedProviderModelConfig"));
        let config_mod =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/config/mod.rs"))
                .expect("read core config exports");
        assert!(!config_mod.contains("AgenaToolMode"));
        assert!(!config_mod.contains("AgenaToolsConfig"));
        assert!(!config_mod.contains("ProviderNetworkConfig"));
        assert!(!config_mod.contains("ProviderProtocolPathsConfig"));
        assert!(!config_mod.contains("ProviderModelDiscoveryConfig"));
        assert!(!config_mod.contains("ProviderSecretSourceConfig"));
        assert!(!config_mod.contains("ProviderGitlabApiAccessConfig"));
        assert!(!config_mod.contains("BedrockSigv4AuthConfig"));
        assert!(!config_mod.contains("ProviderInlineCredentialAuthConfig"));
        assert!(!config_mod.contains("ProviderHttpCredentialAuthConfig"));
        assert!(!config_mod.contains("ProviderSapAiCoreCredentialAuthConfig"));
        assert!(!config_mod.contains("ProviderGitlabCredentialAuthConfig"));
        assert!(!config_mod.contains("ProviderCredentialAuthConfig"));
        assert!(!config_mod.contains("ProviderModelOverlay"));
        assert!(!config_mod.contains("ResolvedProviderModelConfig"));
        let session_mod =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/session/mod.rs"))
                .expect("read session module exports");
        for export in [
            "DEFAULT_SESSION_CACHE_MAX_SESSIONS",
            "DEFAULT_SESSION_CACHE_TTL_SECS",
            "DEFAULT_SESSION_CACHE_MAX_BYTES",
        ] {
            assert!(
                !session_mod.contains(export),
                "legacy session cache export: {export}"
            );
        }
        let wire_message =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/provider/wire_message.rs"))
                .expect("read provider wire message boundary");
        assert!(wire_message.contains("state.clone().into()"));
        assert!(!wire_message.contains("state.gemini_thought_signatures.clone()"));
        assert!(!wire_message.contains("AttachmentSource, ExecutionStatus, Message"));
        for relative in [
            "crates/agena-runtime/src/provider/openai.rs",
            "crates/agena-runtime/src/provider/openai/openai_requests.rs",
            "crates/agena-runtime/src/provider/anthropic.rs",
            "crates/agena-provider/src/anthropic_thinking.rs",
            "crates/agena-runtime/src/provider/amazon_bedrock.rs",
            "crates/agena-runtime/src/provider/gemini.rs",
            "crates/agena-runtime/src/provider/chat_wire.rs",
        ] {
            let source = fs::read_to_string(workspace.join(relative))
                .expect("read provider usage mapping source");
            assert!(
                !source.contains("MessageUsage"),
                "provider mapping must return CompletionUsage directly: {relative}"
            );
        }
    }

    #[test]
    fn process_metrics_are_runtime_owned() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let runtime_metrics =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/metrics.rs"))
                .expect("read runtime metrics");
        assert!(
            runtime_metrics.contains("pub struct RuntimeMetricsSnapshot")
                && runtime_metrics.contains("pub fn runtime_metrics_snapshot()"),
            "runtime must own the process metric snapshot"
        );
        let runtime_lib = fs::read_to_string(workspace.join("crates/agena-runtime/src/lib.rs"))
            .expect("read runtime metric exports");
        let runtime_control = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime_control_service.rs"),
        )
        .expect("read runtime metric control capability");
        assert!(
            runtime_control.contains("fn runtime_metrics(&self) -> crate::RuntimeMetricsSnapshot")
                && !runtime_lib.contains(
                    "pub use metrics::{RuntimeMetricsSnapshot, runtime_metrics_snapshot};"
                ),
            "process telemetry must cross the Runtime control capability rather than a public global helper"
        );
        assert!(
            !workspace.join("crates/agena/src/metrics.rs").exists(),
            "deleted Core monolith must not retain the process metric implementation"
        );
        let api_metrics =
            fs::read_to_string(workspace.join("crates/agena-api-server/src/rest/mod.rs"))
                .expect("read API metrics endpoint");
        assert!(
            api_metrics.contains("let runtime_metrics = state.runtime_metrics();")
                && !api_metrics.contains("agena_runtime::runtime_metrics_snapshot()"),
            "API metrics must consume the Application projection rather than Runtime directly"
        );
        assert!(
            !api_metrics.contains("agena::metrics::"),
            "API metrics must not import Core metrics"
        );
    }

    #[test]
    fn workspace_production_sources_do_not_reopen_config_resolution() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in ["crates", "apps"] {
            let source = collect_rust_sources(&workspace.join(relative))
                .expect("read workspace production sources")
                .join("\n");
            assert!(
                !source.contains("config_resolution()"),
                "production source must use capability-specific snapshot accessors: {relative}"
            );
            if relative == "apps" || relative == "crates" {
                assert!(
                    !source.contains("agena::message::ExecutionStatus"),
                    "domain-owned ExecutionStatus must not re-enter through the core message facade: {relative}"
                );
                assert!(
                    !source.contains("agena::message::MessageSource"),
                    "domain-owned MessageSource must not re-enter through the core message facade: {relative}"
                );
                assert!(
                    !source.contains("agena::config::AgenaToolMode"),
                    "provider-owned AgenaToolMode must not re-enter through the core config facade: {relative}"
                );
                assert!(
                    !source.contains("agena::config::AgenaToolsConfig"),
                    "provider-owned AgenaToolsConfig must not re-enter through the core config facade: {relative}"
                );
                assert!(
                    !source.contains("agena::config::ProviderNetworkConfig"),
                    "provider-owned network configuration must not re-enter through the core config facade: {relative}"
                );
                assert!(
                    !source.contains("agena::config::ProviderProtocolPathsConfig")
                        && !source.contains("agena::config::ProviderModelDiscoveryConfig"),
                    "provider-owned route configuration must not re-enter through the core config facade: {relative}"
                );
                assert!(
                    !source.contains("agena::config::ProviderSecretSourceConfig")
                        && !source.contains("agena::config::ProviderGitlabApiAccessConfig"),
                    "provider-owned secret configuration must not re-enter through the core config facade: {relative}"
                );
                assert!(
                    !source.contains("agena::config::BedrockSigv4AuthConfig"),
                    "provider-owned Bedrock auth configuration must not re-enter through the core config facade: {relative}"
                );
                for forbidden in [
                    "agena::config::ProviderInlineCredentialAuthConfig",
                    "agena::config::ProviderHttpCredentialAuthConfig",
                    "agena::config::ProviderSapAiCoreCredentialAuthConfig",
                    "agena::config::ProviderGitlabCredentialAuthConfig",
                    "agena::config::ProviderCredentialAuthConfig",
                ] {
                    assert!(
                        !source.contains(forbidden),
                        "provider-owned credential auth configuration must not re-enter through the core config facade: {forbidden} ({relative})"
                    );
                }
                assert!(
                    !source.contains("agena::config::ProviderModelOverlay")
                        && !source.contains("agena::config::ResolvedProviderModelConfig"),
                    "provider-owned resolved model configuration must not re-enter through the core config facade: {relative}"
                );
                assert!(
                    !source.contains("agena::message::InteractionNotificationLevel"),
                    "domain-owned notification level must not re-enter through the core message facade: {relative}"
                );
                assert!(
                    !source.contains("agena::message::ToolResultState"),
                    "domain-owned tool result state must not re-enter through the core message facade: {relative}"
                );
                assert!(
                    !source.contains("agena::message::ToolResultDisplay"),
                    "domain-owned tool result display must not re-enter through the core message facade: {relative}"
                );
                assert!(
                    !source.contains("agena::message::OperationError"),
                    "domain-owned operation error must not re-enter through the core message facade: {relative}"
                );
                for forbidden in [
                    "agena::message::UserInputOption",
                    "agena::message::UserInputQuestion",
                    "agena::message::UserInputRequest",
                    "agena::message::UserInputReply",
                    "agena::message::PluginInvocation",
                    "agena::message::ArtifactRef",
                    "agena::message::SearchResultItem",
                    "agena::message::TableColumn",
                    "agena::message::TodoItem",
                ] {
                    assert!(
                        !source.contains(forbidden),
                        "domain-owned value must not re-enter through the core message facade: {forbidden} ({relative})"
                    );
                }
                if relative == "crates" {
                    let application = fs::read_to_string(
                        workspace.join("crates/agena-application/src/service/messages.rs"),
                    )
                    .expect("read application message boundary");
                    assert!(!application.contains("agena::message::PartKind"));
                }
            }
        }
    }

    #[test]
    fn todo_item_is_domain_owned_without_core_facade_definition() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let domain = fs::read_to_string(workspace.join("crates/agena-domain/src/lib.rs"))
            .expect("read domain exports");
        assert!(
            domain.contains("TodoItem")
                && domain.contains("SearchResultItem")
                && domain.contains("ArtifactRef")
                && domain.contains("PluginInvocation")
        );
        let activity =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/message/part/activity.rs"))
                .expect("read core activity values");
        assert!(!activity.contains("pub struct TodoItem"));
        let part =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/message/part/mod.rs"))
                .expect("read core part exports");
        assert!(!part.contains("TodoItem"));
        let tool =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/message/part/tool.rs"))
                .expect("read tool operation block source");
        assert!(tool.contains("SearchResultItem") && tool.contains("TableColumn"));
        assert!(!tool.contains("pub struct SearchResultItem"));
        assert!(!tool.contains("pub struct TableColumn"));
        assert!(!tool.contains("pub struct ArtifactRef"));
        assert!(!tool.contains("pub struct PluginInvocation"));
    }

    #[test]
    fn runtime_crate_source_has_no_legacy_core_reference() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source_root = workspace.join("crates/agena-runtime/src");
        assert_runtime_source_is_core_free(&source_root).expect("runtime source must be core-free");
        let composition = fs::read_to_string(source_root.join("composition.rs"))
            .expect("read runtime composition inputs");
        assert!(composition.contains("ModelCatalogCompositionInputs"));
        assert!(composition.contains("pub(crate) providers"));
        assert!(composition.contains("pub(crate) config_path"));
        assert!(composition.contains("pub(crate) plugins"));
        assert!(composition.contains("pub(crate) database"));
        assert!(composition.contains("PluginCompositionInputs"));
        assert!(composition.contains("pub(crate) plugin_config"));
        assert!(composition.contains("pub(crate) workspace_root"));
        assert!(composition.contains("pub(crate) previous_host"));
        assert!(composition.contains("pub(crate) previous_config"));
        assert!(composition.contains("pub(crate) mcp_manager"));
        assert!(!composition.contains("LspCompositionInputs"));
        assert!(composition.contains("SessionCompositionInputs"));
        for field in [
            "pub(crate) existing",
            "pub(crate) database",
            "pub(crate) providers",
            "pub(crate) plugins",
            "pub(crate) agents",
            "pub(crate) lsp_registry",
            "pub(crate) workspace_root",
            "pub(crate) config",
        ] {
            assert!(
                composition.contains(field),
                "missing session input field: {field}"
            );
        }
        assert!(composition.contains("ToolCompositionInputs"));
        assert!(composition.contains("pub(crate) tool_presentation"));
        assert!(composition.contains("pub(crate) session_manager"));
        assert!(composition.contains("DatabaseCompositionInputs"));
        assert!(composition.contains("pub(crate) database_connection"));
        assert!(composition.contains("pub(crate) database_url"));
        assert!(composition.contains("pub(crate) database_path"));
        assert!(composition.contains("pub(crate) initialize_schema"));
        assert!(composition.contains("pub(crate) tracing"));
        assert!(composition.contains("ModelCatalogRuntimeConfig"));
        assert!(composition.contains("cache_max_age_secs"));
        let runtime_builder =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/runtime/builder.rs"))
                .expect("read runtime database builder");
        assert!(runtime_builder.contains("agena_runtime::connect_runtime_database("));
        assert!(!runtime_builder.contains("async fn connect_database("));
        let runtime_database =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/tracing_config.rs"))
                .expect("read runtime database composition");
        assert!(runtime_database.contains("pub(crate) async fn connect_runtime_database("));
        assert!(runtime_database.contains("initialize_schema(database.as_ref())"));
        let builders = fs::read_to_string(
            workspace.join("crates/agena-runtime/src/runtime/snapshot/builders.rs"),
        )
        .expect("read model catalog builder");
        assert!(builders.contains("ModelCatalogCompositionInputs"));
        assert!(builders.contains("ModelCatalogService::compose_default_optional(database)"));
        let model_catalog_service =
            fs::read_to_string(workspace.join("crates/agena-runtime/src/model_catalog_service.rs"))
                .expect("read Runtime model catalog service");
        assert!(
            model_catalog_service.contains("pub async fn compose_default_optional("),
            "Runtime must own optional model-catalog persistence composition"
        );
        assert!(
            !workspace
                .join("crates/agena-runtime/src/model_catalog/store.rs")
                .exists(),
            "model catalog must not retain its SeaORM repository"
        );
        let catalog_store = fs::read_to_string(
            workspace.join("crates/agena-storage-sqlite/src/model_catalog_repository.rs"),
        )
        .expect("read SQLite model catalog repository");
        assert!(catalog_store.contains("CatalogModelDefinition::from_persisted_json"));
        assert!(catalog_store.contains("definition.to_persisted_json()"));
        assert!(!catalog_store.contains("serde_json::from_value::<CatalogModelDefinition>"));
        assert!(!catalog_store.contains("serde_json::to_value(definition)"));
    }

    #[test]
    fn migrated_runtime_primitives_do_not_return_through_core_facades() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in ["crates", "apps"] {
            assert_no_legacy_runtime_facade_reference(&workspace.join(relative))
                .expect("migrated runtime primitives must be imported from agena-runtime");
        }
    }

    #[test]
    fn core_runtime_has_no_detached_task_primitives() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert_core_runtime_has_no_forbidden_source(
            &workspace.join("crates/agena-runtime/src/runtime"),
        )
        .expect("core runtime must use runtime-owned task lifecycle primitives");
    }

    fn assert_core_runtime_has_no_forbidden_source(directory: &Path) -> anyhow::Result<()> {
        const FORBIDDEN: &[&str] = &[
            "tokio::spawn",
            "tokio::task::JoinHandle",
            "tokio::sync::Mutex",
            "Handle::spawn",
        ];
        for entry in fs::read_dir(directory)
            .with_context(|| format!("failed to read {}", directory.display()))?
        {
            let entry =
                entry.with_context(|| format!("failed to inspect {}", directory.display()))?;
            let path = entry.path();
            if path.is_dir() {
                assert_core_runtime_has_no_forbidden_source(&path)?;
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let source = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            for forbidden in FORBIDDEN {
                assert!(
                    !source.contains(forbidden),
                    "{} contains core-owned task lifecycle primitive `{forbidden}`",
                    path.display()
                );
            }
        }
        Ok(())
    }

    fn assert_no_legacy_runtime_facade_reference(directory: &Path) -> anyhow::Result<()> {
        const FORBIDDEN: &[&str] = &[
            "agena::runtime::RuntimeBackgroundTask",
            "agena::runtime::RuntimeReload",
            "agena::runtime::TaskControl",
            "agena::runtime::SnapshotStore",
            "agena::runtime::RuntimeTaskState",
            "agena::runtime::WatchPathSet",
            "agena::runtime::AbortOnDrop",
            "agena::runtime::build_app_runtime",
        ];
        for entry in fs::read_dir(directory)
            .with_context(|| format!("failed to read {}", directory.display()))?
        {
            let entry =
                entry.with_context(|| format!("failed to inspect {}", directory.display()))?;
            let path = entry.path();
            if path.is_dir() {
                assert_no_legacy_runtime_facade_reference(&path)?;
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let source = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            for forbidden in FORBIDDEN {
                assert!(
                    !source.contains(forbidden),
                    "{} contains migrated runtime primitive through core facade `{forbidden}`",
                    path.display()
                );
            }
        }
        Ok(())
    }

    fn assert_runtime_source_is_core_free(directory: &Path) -> anyhow::Result<()> {
        for entry in fs::read_dir(directory)
            .with_context(|| format!("failed to read {}", directory.display()))?
        {
            let entry =
                entry.with_context(|| format!("failed to inspect {}", directory.display()))?;
            let path = entry.path();
            if path.is_dir() {
                assert_runtime_source_is_core_free(&path)?;
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let source = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            for forbidden in [
                "agena_core",
                "agena-core",
                "use agena::",
                "extern crate agena",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "runtime source {} contains legacy-core reference `{forbidden}`",
                    path.display()
                );
            }
        }
        Ok(())
    }

    #[test]
    fn source_include_detector_accepts_a_normal_module() {
        let root = tempfile::tempdir().expect("temporary source root");
        let source = root.path().join("lib.rs");
        fs::write(source, "mod feature;\n").expect("write source");

        assert_directory_has_no_textual_source_includes(root.path())
            .expect("normal Rust modules are permitted");
    }

    #[test]
    fn source_include_detector_rejects_textual_inclusion() {
        let root = tempfile::tempdir().expect("temporary source root");
        let source = root.path().join("lib.rs");
        fs::write(source, "include_str!(\"README.md\");\n").expect("write source");

        let error = assert_directory_has_no_textual_source_includes(root.path())
            .expect_err("textual source inclusion must be rejected");
        assert!(error.to_string().contains("include_str!"));
    }
}
