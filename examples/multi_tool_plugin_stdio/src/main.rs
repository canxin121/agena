use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct NotesConfig {
    prefix: String,
    uppercase: bool,
}

impl Default for NotesConfig {
    fn default() -> Self {
        Self {
            prefix: "[note] ".to_string(),
            uppercase: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[serde(deny_unknown_fields)]
struct FormatNoteInput {
    #[arg(trim, non_empty)]
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[serde(deny_unknown_fields)]
struct WriteNoteInput {
    #[arg(trim, non_empty)]
    path: String,
    #[arg(trim, non_empty)]
    text: String,
    #[serde(default)]
    append: bool,
}

#[derive(Default, PluginConfigStore)]
struct NotesPlugin {
    #[config(default)]
    config: PluginConfig<NotesConfig>,
}

#[agena_plugin(
    namespace = "example",
    name = "notes",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Multi-tool stdio plugin example for notes formatting and file writes.",
    config,
    display = compact,
    export = stdio
)]
impl NotesPlugin {
    fn config(&self) -> NotesConfig {
        self.config.get().cloned().unwrap_or_default()
    }

    fn render(&self, text: &str) -> String {
        let config = self.config();
        let rendered = format!("{}{}", config.prefix, text);
        if config.uppercase {
            rendered.to_uppercase()
        } else {
            rendered
        }
    }

    #[tool(
        name = "format",
        summary = "Format text with the configured notes prefix.",
        help = "Formats text using this plugin's runtime config. The streaming path emits the formatted text in line-sized chunks.",
        read_only,
        streaming,
        concurrency_safe
    )]
    async fn format(&self, input: &FormatNoteInput) -> Result<ToolInvokeOutput> {
        let rendered = self.render(input.text.as_str());
        Ok(ToolInvokeOutput::from_parts(
            "Format note",
            rendered.clone(),
            Some(json!({ "rendered": rendered })),
            Default::default(),
            Vec::new(),
        ))
    }

    #[stream(format)]
    async fn format_stream(
        &self,
        sink: ToolStreamSink,
        input: &FormatNoteInput,
    ) -> Result<ToolStreamEnd> {
        let rendered = self.render(input.text.as_str());
        for chunk in rendered.split_inclusive('\n') {
            sink.text(chunk).await;
        }
        Ok(ToolStreamEnd::text(sink.stream_id().to_string(), rendered))
    }

    #[tool(
        name = "write",
        summary = "Write formatted text to a file.",
        help = "Writes the formatted text to the provided path. Path permission is supplied dynamically by this tool's permission handler.",
        mutating,
        filesystem_write,
        permission(paths = write_permission_paths)
    )]
    async fn write(&self, input: &WriteNoteInput) -> Result<ToolInvokeOutput> {
        let rendered = self.render(input.text.as_str());
        let path = PathBuf::from(input.path.as_str());
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|err| {
                PluginError::new(format!("failed to create parent directory: {err}"))
            })?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(input.append)
            .truncate(!input.append)
            .open(&path)
            .map_err(|err| PluginError::new(format!("failed to open note file: {err}")))?;
        file.write_all(rendered.as_bytes())
            .map_err(|err| PluginError::new(format!("failed to write note file: {err}")))?;

        Ok(ToolInvokeOutput::from_parts(
            "Write note",
            format!("wrote {}", path.display()),
            Some(json!({
                "path": input.path,
                "append": input.append,
                "bytes": rendered.len()
            })),
            Default::default(),
            Vec::new(),
        ))
    }

    async fn write_permission_paths(&self, input: &WriteNoteInput) -> Result<Vec<PathRequest>> {
        Ok(vec![PathRequest::write(input.path.clone())])
    }
}
