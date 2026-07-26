//! Direct image generation/edit tools backed by the selected provider route.

use std::sync::{Arc, OnceLock};

use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{
    HostClient, HostImageExecuteRequest, HostImageInput, HostImageOperation,
};
use agena_plugin_host::sdk::{
    HostCapability, InitContext, InitOutcome, PathRequest, Result as SdkResult, ToolInvokeContext,
    ToolInvokeOutput,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const IMAGE_PLUGIN_ID: &str = "agena.image";

pub(crate) struct ImagePlugin {
    host: OnceLock<Arc<dyn HostClient>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ImageBackground {
    Auto,
    Opaque,
    Transparent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
enum ImageSize {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "1024x1024")]
    Square,
    #[serde(rename = "1536x1024")]
    Landscape,
    #[serde(rename = "1024x1536")]
    Portrait,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ImageQuality {
    Auto,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ImageModeration {
    Auto,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct ImageOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<ImageBackground>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<ImageSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quality: Option<ImageQuality>,
    #[serde(skip_serializing_if = "Option::is_none")]
    moderation: Option<ImageModeration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("prompt"), non_empty("prompt"), max_chars("prompt", 32000))]
#[serde(deny_unknown_fields)]
struct ImageGenerateInput {
    /// Detailed description of the image to create.
    prompt: String,
    #[serde(default, flatten)]
    options: ImageOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "images[]"),
    non_empty("prompt", "images[]"),
    max_chars("prompt", 32000),
    min_items("images", 1),
    max_items("images", 16)
)]
#[serde(deny_unknown_fields)]
struct ImageEditInput {
    /// Description of the requested transformation.
    prompt: String,
    /// Permitted local image paths used as edit references.
    images: Vec<String>,
    #[serde(default, flatten)]
    options: ImageOptions,
}

impl ImagePlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: OnceLock::new(),
        }
    }

    fn host(&self) -> SdkResult<&Arc<dyn HostClient>> {
        self.host
            .get()
            .ok_or_else(|| PluginError::new("image plugin invoked before init"))
    }

    async fn execute(
        &self,
        context: &ToolInvokeContext<'_>,
        operation: HostImageOperation,
        prompt: String,
        inputs: Vec<HostImageInput>,
        options: ImageOptions,
    ) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .host()?
            .image_execute(HostImageExecuteRequest {
                session_id: Some(context.session_id),
                operation,
                prompt,
                inputs,
                background: option_value(options.background)?,
                size: option_value(options.size)?,
                quality: option_value(options.quality)?,
                moderation: option_value(options.moderation)?,
            })
            .await?;
        let operation_label = match response.operation {
            HostImageOperation::Generate => "generated",
            HostImageOperation::Edit => "edited",
        };
        let route = match response.adapter_id.as_deref() {
            Some(adapter) => format!("{}/{}/{}", response.provider_id, adapter, response.model_id),
            None => format!("{}/{}", response.provider_id, response.model_id),
        };
        let payload = serde_json::json!({
            "operation": response.operation,
            "provider_id": response.provider_id,
            "adapter_id": response.adapter_id,
            "model_id": response.model_id,
            "revised_prompt": response.revised_prompt,
            "artifacts": response.attachments.iter().map(|attachment| serde_json::json!({
                "mime": attachment.mime,
                "filename": attachment.filename,
                "size_bytes": attachment.size_bytes,
                "sha256": attachment.sha256,
                "source": attachment.source,
            })).collect::<Vec<_>>(),
        });
        Ok(ToolInvokeOutput::from_parts(
            format!("image {operation_label}"),
            format!(
                "Image {operation_label} through active route {route}; persisted {} managed attachment(s).",
                response.attachments.len()
            ),
            Some(payload),
            std::collections::BTreeMap::from([
                ("provider_id".to_owned(), response.provider_id),
                ("model_id".to_owned(), response.model_id),
                (
                    "attachment_count".to_owned(),
                    response.attachments.len().to_string(),
                ),
            ]),
            response.attachments,
        ))
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "image",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Generate or edit images through the active provider/model route and persist managed attachments.",
    display = detailed
)]
impl ImagePlugin {
    #[hook(init)]
    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::new("image plugin initialized more than once"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        summary = "Generate an image using the active route's real direct image API.",
        help = "The selected provider/model route must explicitly enable provider-hosted image_generation and its adapter must implement Agena's direct image port. Output is returned only after it has been copied into the managed artifact store with MIME, size, and SHA-256 metadata.",
        mutating,
        display = detailed,
        capabilities(HostCapability::ImageGeneration),
        examples(r#"{"prompt":"A watercolor map of a floating city","size":"1536x1024","quality":"high"}"#)
    )]
    async fn generate(
        &self,
        context: &ToolInvokeContext<'_>,
        input: ImageGenerateInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.execute(
            context,
            HostImageOperation::Generate,
            input.prompt,
            Vec::new(),
            input.options,
        )
        .await
    }

    #[tool(
        summary = "Edit permitted local images using the active route's real direct image API.",
        help = "Every source path is permission-checked, read into a bounded attachment, and verified by image signature, MIME, decoded size, and SHA-256 before crossing the provider boundary. Remote URLs and provider file ids are not accepted as edit inputs.",
        mutating,
        filesystem_read,
        display = detailed,
        capabilities(HostCapability::ImageGeneration),
        path(requests = input.images.iter().cloned().map(PathRequest::read).collect::<Vec<_>>()),
        examples(r#"{"prompt":"Replace the sky with an aurora","images":["assets/source.png"]}"#)
    )]
    async fn edit(
        &self,
        context: &ToolInvokeContext<'_>,
        input: ImageEditInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let inputs = input
            .images
            .into_iter()
            .map(|path| HostImageInput::Path { path })
            .collect();
        self.execute(
            context,
            HostImageOperation::Edit,
            input.prompt,
            inputs,
            input.options,
        )
        .await
    }
}

fn option_value<T: Serialize>(value: Option<T>) -> SdkResult<Option<String>> {
    value
        .map(|value| {
            serde_json::to_value(value)
                .map_err(|error| PluginError::new(error.to_string()))
                .and_then(|value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| PluginError::new("image option did not serialize as text"))
                })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::{HostCapability, Plugin};

    use super::ImagePlugin;

    #[test]
    fn manifest_exposes_real_host_backed_image_tools() {
        let manifest = ImagePlugin::new().manifest();
        assert_eq!(manifest.tools.len(), 2);
        let generate = manifest
            .tools
            .iter()
            .find(|tool| tool.name == "generate")
            .expect("generate tool");
        let edit = manifest
            .tools
            .iter()
            .find(|tool| tool.name == "edit")
            .expect("edit tool");
        assert!(
            generate
                .capabilities
                .contains(&HostCapability::ImageGeneration)
        );
        assert!(edit.capabilities.contains(&HostCapability::ImageGeneration));
        assert_eq!(edit.permissions.input_paths.len(), 0);
    }

    #[tokio::test]
    async fn edit_requests_read_permission_for_every_input_path() {
        let plugin = ImagePlugin::new();
        let paths = Plugin::permission_paths(
            &plugin,
            "edit",
            &serde_json::json!({
                "prompt": "fixture",
                "images": ["one.png", "two.webp"]
            }),
        )
        .await
        .expect("permission paths");
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].path, "one.png");
        assert_eq!(paths[1].path, "two.webp");
        assert!(
            paths
                .iter()
                .all(|path| path.kind == agena_plugin_host::sdk::PathKind::Read)
        );
    }
}
