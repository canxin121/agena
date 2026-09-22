//! Model-visible vendor capabilities are hosted services, not a second local
//! executor. Retired names remain here solely for migration diagnostics; they
//! are not aliases, registrations, or instructions to execute old callbacks.

/// Public names make execution location explicit. API operation names stay
/// separate so renaming an Agena tool never renames a vendor protocol field.
#[derive(Debug, Clone, Copy)]
pub struct CloudProviderTool {
    pub name: &'static str,
    pub previous_name: &'static str,
    pub provider: &'static str,
    pub operation: &'static str,
    pub provider_label: &'static str,
    pub title: &'static str,
}

pub const CLOUD_TOOLS: &[CloudProviderTool] = &[
    CloudProviderTool {
        previous_name: "chatgpt.web_search",
        name: "chatgpt.cloud_web_search",
        provider: "chatgpt",
        operation: "web_search",
        provider_label: "OpenAI",
        title: "Search web in OpenAI cloud",
    },
    CloudProviderTool {
        previous_name: "chatgpt.file_search",
        name: "chatgpt.cloud_file_search",
        provider: "chatgpt",
        operation: "file_search",
        provider_label: "OpenAI",
        title: "Search files in OpenAI cloud",
    },
    CloudProviderTool {
        previous_name: "chatgpt.code_interpreter",
        name: "chatgpt.cloud_code_interpreter",
        provider: "chatgpt",
        operation: "code_interpreter",
        provider_label: "OpenAI",
        title: "Run Python in OpenAI cloud",
    },
    CloudProviderTool {
        previous_name: "chatgpt.image_generation",
        name: "chatgpt.cloud_image_generation",
        provider: "chatgpt",
        operation: "image_generation",
        provider_label: "OpenAI",
        title: "Generate image in OpenAI cloud",
    },
    CloudProviderTool {
        previous_name: "chatgpt.image_edit",
        name: "chatgpt.cloud_image_edit",
        provider: "chatgpt",
        operation: "image_edit",
        provider_label: "OpenAI",
        title: "Edit image in OpenAI cloud",
    },
    CloudProviderTool {
        previous_name: "chatgpt.shell",
        name: "chatgpt.cloud_shell",
        provider: "chatgpt",
        operation: "shell",
        provider_label: "OpenAI",
        title: "Run shell in OpenAI cloud",
    },
    CloudProviderTool {
        previous_name: "claude.web_search",
        name: "claude.cloud_web_search",
        provider: "claude",
        operation: "web_search",
        provider_label: "Anthropic",
        title: "Search web in Anthropic cloud",
    },
    CloudProviderTool {
        previous_name: "claude.web_fetch",
        name: "claude.cloud_web_fetch",
        provider: "claude",
        operation: "web_fetch",
        provider_label: "Anthropic",
        title: "Fetch web content in Anthropic cloud",
    },
    CloudProviderTool {
        previous_name: "claude.code_execution",
        name: "claude.cloud_code_execution",
        provider: "claude",
        operation: "code_execution",
        provider_label: "Anthropic",
        title: "Run code in Anthropic cloud",
    },
    CloudProviderTool {
        previous_name: "claude.advisor",
        name: "claude.cloud_advisor",
        provider: "claude",
        operation: "advisor",
        provider_label: "Anthropic",
        title: "Consult advisor in Anthropic cloud",
    },
    CloudProviderTool {
        previous_name: "gemini.code_execution",
        name: "gemini.cloud_code_execution",
        provider: "gemini",
        operation: "code_execution",
        provider_label: "Google",
        title: "Run code in Google cloud",
    },
    CloudProviderTool {
        previous_name: "gemini.url_context",
        name: "gemini.cloud_url_context",
        provider: "gemini",
        operation: "url_context",
        provider_label: "Google",
        title: "Read URL context in Google cloud",
    },
    CloudProviderTool {
        previous_name: "gemini.google_search",
        name: "gemini.cloud_google_search",
        provider: "gemini",
        operation: "google_search",
        provider_label: "Google",
        title: "Search web in Google cloud",
    },
    CloudProviderTool {
        previous_name: "gemini.file_search",
        name: "gemini.cloud_file_search",
        provider: "gemini",
        operation: "file_search",
        provider_label: "Google",
        title: "Search files in Google cloud",
    },
    CloudProviderTool {
        previous_name: "gemini.google_maps",
        name: "gemini.cloud_google_maps",
        provider: "gemini",
        operation: "google_maps",
        provider_label: "Google",
        title: "Search maps in Google cloud",
    },
    CloudProviderTool {
        previous_name: "gemini.image_generation",
        name: "gemini.cloud_image_generation",
        provider: "gemini",
        operation: "image_generation",
        provider_label: "Google",
        title: "Generate image in Google cloud",
    },
    CloudProviderTool {
        previous_name: "gemini.image_edit",
        name: "gemini.cloud_image_edit",
        provider: "gemini",
        operation: "image_edit",
        provider_label: "Google",
        title: "Edit image in Google cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_image_understanding",
        previous_name: "chatgpt.image_understanding",
        provider: "chatgpt",
        operation: "image_understanding",
        provider_label: "OpenAI",
        title: "Analyze images in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_document_understanding",
        previous_name: "chatgpt.document_understanding",
        provider: "chatgpt",
        operation: "document_understanding",
        provider_label: "OpenAI",
        title: "Analyze documents in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_file_upload",
        previous_name: "chatgpt.file_upload",
        provider: "chatgpt",
        operation: "file_upload",
        provider_label: "OpenAI",
        title: "Upload file in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_file_status",
        previous_name: "chatgpt.file_status",
        provider: "chatgpt",
        operation: "file_status",
        provider_label: "OpenAI",
        title: "Inspect uploaded file in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_file_delete",
        previous_name: "chatgpt.file_delete",
        provider: "chatgpt",
        operation: "file_delete",
        provider_label: "OpenAI",
        title: "Delete uploaded file in OpenAI cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_image_understanding",
        previous_name: "claude.image_understanding",
        provider: "claude",
        operation: "image_understanding",
        provider_label: "Anthropic",
        title: "Analyze images in Anthropic cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_document_understanding",
        previous_name: "claude.document_understanding",
        provider: "claude",
        operation: "document_understanding",
        provider_label: "Anthropic",
        title: "Analyze documents in Anthropic cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_file_upload",
        previous_name: "claude.file_upload",
        provider: "claude",
        operation: "file_upload",
        provider_label: "Anthropic",
        title: "Upload file in Anthropic cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_file_status",
        previous_name: "claude.file_status",
        provider: "claude",
        operation: "file_status",
        provider_label: "Anthropic",
        title: "Inspect uploaded file in Anthropic cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_file_delete",
        previous_name: "claude.file_delete",
        provider: "claude",
        operation: "file_delete",
        provider_label: "Anthropic",
        title: "Delete uploaded file in Anthropic cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_image_understanding",
        previous_name: "gemini.image_understanding",
        provider: "gemini",
        operation: "image_understanding",
        provider_label: "Google",
        title: "Analyze images in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_document_understanding",
        previous_name: "gemini.document_understanding",
        provider: "gemini",
        operation: "document_understanding",
        provider_label: "Google",
        title: "Analyze documents in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_file_upload",
        previous_name: "gemini.file_upload",
        provider: "gemini",
        operation: "file_upload",
        provider_label: "Google",
        title: "Upload file in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_file_status",
        previous_name: "gemini.file_status",
        provider: "gemini",
        operation: "file_status",
        provider_label: "Google",
        title: "Inspect uploaded file in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_file_delete",
        previous_name: "gemini.file_delete",
        provider: "gemini",
        operation: "file_delete",
        provider_label: "Google",
        title: "Delete uploaded file in Google cloud",
    },
];

pub const HOSTED_TOOLS: &[&str] = &[
    "chatgpt.cloud_web_search",
    "chatgpt.cloud_file_search",
    "chatgpt.cloud_code_interpreter",
    "chatgpt.cloud_image_generation",
    "chatgpt.cloud_image_edit",
    "chatgpt.cloud_shell",
    "claude.cloud_web_search",
    "claude.cloud_web_fetch",
    "claude.cloud_code_execution",
    "claude.cloud_advisor",
    "gemini.cloud_code_execution",
    "gemini.cloud_url_context",
    "gemini.cloud_google_search",
    "gemini.cloud_file_search",
    "gemini.cloud_google_maps",
    "gemini.cloud_image_generation",
    "gemini.cloud_image_edit",
    "chatgpt.cloud_image_understanding",
    "chatgpt.cloud_document_understanding",
    "chatgpt.cloud_file_upload",
    "chatgpt.cloud_file_status",
    "chatgpt.cloud_file_delete",
    "claude.cloud_image_understanding",
    "claude.cloud_document_understanding",
    "claude.cloud_file_upload",
    "claude.cloud_file_status",
    "claude.cloud_file_delete",
    "gemini.cloud_image_understanding",
    "gemini.cloud_document_understanding",
    "gemini.cloud_file_upload",
    "gemini.cloud_file_status",
    "gemini.cloud_file_delete",
];

#[derive(Debug, Clone, Copy)]
pub struct RetiredProviderTool {
    pub name: &'static str,
    pub reason: &'static str,
    pub alternatives: &'static [&'static str],
}

pub const RETIRED_TOOLS: &[RetiredProviderTool] = &[
    RetiredProviderTool {
        name: "chatgpt.computer",
        reason: "computer actions were executed by the application, not by OpenAI",
        alternatives: &["web.browser_open", "web.browser_click", "web.browser_type"],
    },
    RetiredProviderTool {
        name: "chatgpt.computer_use_preview",
        reason: "legacy computer actions required application-side execution",
        alternatives: &["web.browser_open", "web.browser_click", "web.browser_type"],
    },
    RetiredProviderTool {
        name: "chatgpt.local_shell",
        reason: "local shell execution already belongs to the native shell tools",
        alternatives: &["shell.run", "shell.write"],
    },
    RetiredProviderTool {
        name: "chatgpt.apply_patch",
        reason: "this declaration returned a patch for the application to execute",
        alternatives: &["fs.apply_patch"],
    },
    RetiredProviderTool {
        name: "chatgpt.function",
        reason: "a function declaration is not a hosted implementation",
        alternatives: &["tasks.run"],
    },
    RetiredProviderTool {
        name: "chatgpt.custom",
        reason: "a custom tool declaration is not a hosted implementation",
        alternatives: &["tasks.run"],
    },
    RetiredProviderTool {
        name: "chatgpt.namespace",
        reason: "a namespace only groups tool declarations and performs no execution",
        alternatives: &[],
    },
    RetiredProviderTool {
        name: "claude.bash",
        reason: "Bash client callbacks required application-side execution",
        alternatives: &["shell.run", "shell.write"],
    },
    RetiredProviderTool {
        name: "claude.computer",
        reason: "computer client callbacks required application-side execution",
        alternatives: &["web.browser_open", "web.browser_click", "web.browser_type"],
    },
    RetiredProviderTool {
        name: "claude.memory",
        reason: "the client memory tool was not an Anthropic-hosted memory store",
        alternatives: &["memory.get", "memory.write", "memory.search"],
    },
    RetiredProviderTool {
        name: "claude.text_editor",
        reason: "client editing already belongs to native filesystem tools",
        alternatives: &["fs.read", "fs.replace", "fs.apply_patch"],
    },
    RetiredProviderTool {
        name: "gemini.computer_use",
        reason: "computer actions required execution in an application-owned environment",
        alternatives: &["web.browser_open", "web.browser_click", "web.browser_type"],
    },
    RetiredProviderTool {
        name: "gemini.function",
        reason: "a function declaration does not run its implementation on Google servers",
        alternatives: &["tasks.run"],
    },
    RetiredProviderTool {
        name: "chatgpt.web_search_preview",
        reason: "the legacy preview entry was consolidated into the hosted web_search capability",
        alternatives: &["chatgpt.cloud_web_search"],
    },
    RetiredProviderTool {
        name: "chatgpt.mcp",
        reason: "remote MCP executes on its configured server, not necessarily on the model vendor's infrastructure",
        alternatives: &["mcp.tools.call", "mcp.tools.search"],
    },
    RetiredProviderTool {
        name: "claude.mcp_toolset",
        reason: "remote MCP is a connector, not an Anthropic-hosted business implementation",
        alternatives: &["mcp.tools.call", "mcp.tools.search"],
    },
    RetiredProviderTool {
        name: "gemini.mcp_server",
        reason: "remote MCP is a connector, not a Google-hosted business implementation",
        alternatives: &["mcp.tools.call", "mcp.tools.search"],
    },
    RetiredProviderTool {
        name: "chatgpt.programmatic_tool_calling",
        reason: "programmatic orchestration is not exposed as a standalone vendor business tool; no local fallback is provided",
        alternatives: &["tasks.run"],
    },
    RetiredProviderTool {
        name: "chatgpt.tool_search",
        reason: "vendor tool discovery is orchestration, not a standalone business tool; use Agena's Tool API discovery",
        alternatives: &[],
    },
    RetiredProviderTool {
        name: "claude.tool_search_bm25",
        reason: "vendor tool discovery is orchestration, not a standalone business tool; use Agena's Tool API discovery",
        alternatives: &[],
    },
    RetiredProviderTool {
        name: "claude.tool_search_regex",
        reason: "vendor tool discovery is orchestration, not a standalone business tool; use Agena's Tool API discovery",
        alternatives: &[],
    },
    RetiredProviderTool {
        name: "gemini.retrieval",
        reason: "this declaration is not verified against the public Interactions tool contract",
        alternatives: &["gemini.cloud_file_search", "gemini.cloud_google_search"],
    },
    RetiredProviderTool {
        name: "fs.view_image",
        reason: "filesystem preview is not cloud image understanding; choose the provider explicitly or attach the image in the composer",
        alternatives: &[
            "chatgpt.cloud_image_understanding",
            "claude.cloud_image_understanding",
            "gemini.cloud_image_understanding",
        ],
    },
];

fn matches_identity(requested: &str, compact: &str) -> bool {
    if requested.strip_prefix("agena.").unwrap_or(requested) == compact {
        return true;
    }
    let Some(wire) = requested.strip_prefix("agena_") else {
        return false;
    };
    let Some((provider, action)) = compact.split_once('.') else {
        return false;
    };
    wire.strip_prefix(provider)
        .and_then(|value| value.strip_prefix('_'))
        == Some(action)
}

pub fn cloud_tool(name: &str) -> Option<&'static CloudProviderTool> {
    CLOUD_TOOLS
        .iter()
        .find(|tool| matches_identity(name, tool.name))
}

pub fn cloud_operation(provider: &str, operation: &str) -> Option<&'static CloudProviderTool> {
    CLOUD_TOOLS
        .iter()
        .find(|tool| tool.provider == provider && tool.operation == operation)
}

/// Presentation only: old receipts keep their existing operation-specific view.
/// This is not an invocation alias and does not forward old calls.
pub fn operation_identity(name: &str) -> &str {
    cloud_tool(name)
        .map(|tool| tool.previous_name)
        .unwrap_or(name)
}

pub fn renamed(name: &str) -> Option<&'static CloudProviderTool> {
    CLOUD_TOOLS
        .iter()
        .find(|tool| matches_identity(name, tool.previous_name))
}

pub fn retired(name: &str) -> Option<&'static RetiredProviderTool> {
    RETIRED_TOOLS
        .iter()
        .find(|tool| matches_identity(name, tool.name))
}

impl CloudProviderTool {
    pub fn migration_message(&self) -> String {
        format!(
            "'{}' was renamed to '{}' for {} cloud execution. This call was not redirected automatically; no network request or local action ran. Read tools_help for the new name before a separate authorized call.",
            self.previous_name, self.name, self.provider_label
        )
    }
}

impl RetiredProviderTool {
    pub fn message(&self) -> String {
        let next = if self.alternatives.is_empty() {
            "Use tools_search and tools_help to choose an available capability.".to_owned()
        } else {
            format!(
                "Choose and authorize a separate call using tools_help for: {}.",
                self.alternatives.join(", ")
            )
        };
        format!(
            "Provider tool '{}' was retired: {}. {next} This call performed no network request or local action. It was not redirected automatically.",
            self.name, self.reason
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hosted_and_retired_catalogues_are_disjoint_and_exact() {
        let mut all = std::collections::BTreeSet::new();
        assert_eq!(HOSTED_TOOLS.len(), 32);
        assert_eq!(RETIRED_TOOLS.len(), 23);
        for name in HOSTED_TOOLS {
            assert!(all.insert(*name));
            assert!(retired(name).is_none());
        }
        for entry in RETIRED_TOOLS {
            assert!(all.insert(entry.name));
            for name in [
                entry.name.to_owned(),
                format!("agena.{}", entry.name),
                format!("agena_{}", entry.name.replacen('.', "_", 1)),
            ] {
                assert_eq!(retired(&name).unwrap().name, entry.name);
                assert!(retired(&name).unwrap().message().contains("not redirected"));
            }
        }
        for name in [
            "shell.run",
            "fs.apply_patch",
            "mcp.tools.call",
            "other.claude.bash",
            "Claude.bash",
        ] {
            assert!(retired(name).is_none());
        }
    }
}

#[cfg(test)]
mod cloud_name_tests {
    use super::*;
    #[test]
    fn cloud_names_operations_and_migration_targets_are_one_to_one() {
        assert_eq!(CLOUD_TOOLS.len(), HOSTED_TOOLS.len());
        let mut names = std::collections::BTreeSet::new();
        let mut old = std::collections::BTreeSet::new();
        for tool in CLOUD_TOOLS {
            assert!(names.insert(tool.name));
            assert!(old.insert(tool.previous_name));
            assert!(HOSTED_TOOLS.contains(&tool.name));
            assert!(tool.name.contains(".cloud_"));
            assert!(tool.title.contains("cloud"));
            assert!(tool.title.contains(tool.provider_label));
            assert_eq!(
                cloud_operation(tool.provider, tool.operation).unwrap().name,
                tool.name
            );
            for name in [
                tool.name.to_owned(),
                format!("agena.{}", tool.name),
                format!("agena_{}", tool.name.replacen('.', "_", 1)),
            ] {
                assert_eq!(cloud_tool(&name).unwrap().name, tool.name);
                assert_eq!(operation_identity(&name), tool.previous_name);
                assert!(renamed(&name).is_none());
                assert!(retired(&name).is_none());
            }
            for name in [
                tool.previous_name.to_owned(),
                format!("agena.{}", tool.previous_name),
                format!("agena_{}", tool.previous_name.replacen('.', "_", 1)),
            ] {
                let migration = renamed(&name).unwrap();
                let message = migration.migration_message();
                assert!(
                    message
                        .chars()
                        .take(220)
                        .collect::<String>()
                        .contains("not redirected automatically")
                );
                assert!(
                    message.len() <= 320,
                    "migration hints must remain readable in compact tool errors"
                );
                assert_eq!(migration.name, tool.name);
                assert!(
                    migration
                        .migration_message()
                        .contains("not redirected automatically")
                );
                assert!(cloud_tool(&name).is_none());
            }
        }
        for name in [
            "shell.run",
            "web.search",
            "fs.read",
            "mcp.tools.call",
            "my.chatgpt.cloud_shell",
            "chatgpt.cloud_unknown",
            "ChatGPT.cloud_shell",
        ] {
            assert!(cloud_tool(name).is_none());
            assert!(renamed(name).is_none());
            assert_eq!(operation_identity(name), name);
        }
    }
}
