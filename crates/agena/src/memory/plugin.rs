use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::MemoryConfig;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginManifest, Result as SdkResult,
};
use crate::plugin::{ChatSystemTransformInput, ChatSystemTransformPatch};

use super::prompt::build_memory_prompt_section;

pub struct MemoryPlugin {
    config: MemoryConfig,
    workspace_root: std::sync::OnceLock<PathBuf>,
}

impl MemoryPlugin {
    pub fn new(config: MemoryConfig) -> Self {
        Self {
            config,
            workspace_root: std::sync::OnceLock::new(),
        }
    }
}

#[async_trait]
impl Plugin for MemoryPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-memory", env!("CARGO_PKG_VERSION"))
            .description("Persistent file-based memory injected into every conversation.")
            .hooks(HookSubscription::INIT | HookSubscription::CHAT_SYSTEM_TRANSFORM)
            .build()
    }

    async fn init(&self, ctx: InitContext, _host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let _ = self.workspace_root.set(ctx.workspace_root);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn chat_system_transform(
        &self,
        input: ChatSystemTransformInput,
    ) -> SdkResult<Option<ChatSystemTransformPatch>> {
        let _ = input;
        let workspace_root = match self.workspace_root.get() {
            Some(p) => p.clone(),
            None => return Ok(None),
        };

        let memory_section = build_memory_prompt_section(&workspace_root);
        let project_section = if self.config.project_instructions.enabled {
            let mut layers = Vec::new();
            if self.config.project_instructions.include_global
                && let Some(global) = super::discover_global()
            {
                layers.push(global);
            }
            layers.extend(super::discover(&workspace_root));
            super::render_section(&layers)
        } else {
            None
        };

        let mut appended = format!("\n\n{}", memory_section);
        if let Some(project) = project_section {
            appended.push_str("\n\n");
            appended.push_str(&project);
        }

        Ok(Some(ChatSystemTransformPatch {
            append: Some(appended),
            ..Default::default()
        }))
    }
}
