//! Model-visible vendor capabilities are hosted services, not a second local
//! executor. Only the current public cloud-tool identities are represented.

/// Public names make execution location explicit. API operation names stay
/// separate so renaming an Agena tool never renames a vendor protocol field.
#[derive(Debug, Clone, Copy)]
pub struct CloudProviderTool {
    pub name: &'static str,
    pub provider: &'static str,
    pub operation: &'static str,
    pub provider_label: &'static str,
    pub title: &'static str,
}

pub const CLOUD_TOOLS: &[CloudProviderTool] = &[
    CloudProviderTool {
        name: "chatgpt.cloud_web_search",
        provider: "chatgpt",
        operation: "web_search",
        provider_label: "OpenAI",
        title: "Search web in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_file_search",
        provider: "chatgpt",
        operation: "file_search",
        provider_label: "OpenAI",
        title: "Search files in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_code_interpreter",
        provider: "chatgpt",
        operation: "code_interpreter",
        provider_label: "OpenAI",
        title: "Run Python in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_image_generation",
        provider: "chatgpt",
        operation: "image_generation",
        provider_label: "OpenAI",
        title: "Generate image in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_image_edit",
        provider: "chatgpt",
        operation: "image_edit",
        provider_label: "OpenAI",
        title: "Edit image in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_shell",
        provider: "chatgpt",
        operation: "shell",
        provider_label: "OpenAI",
        title: "Run shell in OpenAI cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_web_search",
        provider: "claude",
        operation: "web_search",
        provider_label: "Anthropic",
        title: "Search web in Anthropic cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_web_fetch",
        provider: "claude",
        operation: "web_fetch",
        provider_label: "Anthropic",
        title: "Fetch web content in Anthropic cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_code_execution",
        provider: "claude",
        operation: "code_execution",
        provider_label: "Anthropic",
        title: "Run code in Anthropic cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_advisor",
        provider: "claude",
        operation: "advisor",
        provider_label: "Anthropic",
        title: "Consult advisor in Anthropic cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_code_execution",
        provider: "gemini",
        operation: "code_execution",
        provider_label: "Google",
        title: "Run code in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_url_context",
        provider: "gemini",
        operation: "url_context",
        provider_label: "Google",
        title: "Read URL context in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_google_search",
        provider: "gemini",
        operation: "google_search",
        provider_label: "Google",
        title: "Search web in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_file_search",
        provider: "gemini",
        operation: "file_search",
        provider_label: "Google",
        title: "Search files in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_google_maps",
        provider: "gemini",
        operation: "google_maps",
        provider_label: "Google",
        title: "Search maps in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_image_generation",
        provider: "gemini",
        operation: "image_generation",
        provider_label: "Google",
        title: "Generate image in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_image_edit",
        provider: "gemini",
        operation: "image_edit",
        provider_label: "Google",
        title: "Edit image in Google cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_image_understanding",
        provider: "chatgpt",
        operation: "image_understanding",
        provider_label: "OpenAI",
        title: "Analyze images in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_document_understanding",
        provider: "chatgpt",
        operation: "document_understanding",
        provider_label: "OpenAI",
        title: "Analyze documents in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_file_upload",
        provider: "chatgpt",
        operation: "file_upload",
        provider_label: "OpenAI",
        title: "Upload file in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_file_status",
        provider: "chatgpt",
        operation: "file_status",
        provider_label: "OpenAI",
        title: "Inspect uploaded file in OpenAI cloud",
    },
    CloudProviderTool {
        name: "chatgpt.cloud_file_delete",
        provider: "chatgpt",
        operation: "file_delete",
        provider_label: "OpenAI",
        title: "Delete uploaded file in OpenAI cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_image_understanding",
        provider: "claude",
        operation: "image_understanding",
        provider_label: "Anthropic",
        title: "Analyze images in Anthropic cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_document_understanding",
        provider: "claude",
        operation: "document_understanding",
        provider_label: "Anthropic",
        title: "Analyze documents in Anthropic cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_file_upload",
        provider: "claude",
        operation: "file_upload",
        provider_label: "Anthropic",
        title: "Upload file in Anthropic cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_file_status",
        provider: "claude",
        operation: "file_status",
        provider_label: "Anthropic",
        title: "Inspect uploaded file in Anthropic cloud",
    },
    CloudProviderTool {
        name: "claude.cloud_file_delete",
        provider: "claude",
        operation: "file_delete",
        provider_label: "Anthropic",
        title: "Delete uploaded file in Anthropic cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_image_understanding",
        provider: "gemini",
        operation: "image_understanding",
        provider_label: "Google",
        title: "Analyze images in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_document_understanding",
        provider: "gemini",
        operation: "document_understanding",
        provider_label: "Google",
        title: "Analyze documents in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_file_upload",
        provider: "gemini",
        operation: "file_upload",
        provider_label: "Google",
        title: "Upload file in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_file_status",
        provider: "gemini",
        operation: "file_status",
        provider_label: "Google",
        title: "Inspect uploaded file in Google cloud",
    },
    CloudProviderTool {
        name: "gemini.cloud_file_delete",
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

/// Normalize any current registered identity spelling to the compact public name.
pub fn canonical_cloud_identity(name: &str) -> &str {
    cloud_tool(name).map(|tool| tool.name).unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_catalogue_contains_only_current_names() {
        assert_eq!(CLOUD_TOOLS.len(), 32);
        assert_eq!(HOSTED_TOOLS.len(), 32);
        let mut names = std::collections::BTreeSet::new();
        for tool in CLOUD_TOOLS {
            assert!(names.insert(tool.name));
            assert!(tool.name.contains(".cloud_"));
            assert!(HOSTED_TOOLS.contains(&tool.name));
            assert_eq!(
                cloud_operation(tool.provider, tool.operation).unwrap().name,
                tool.name
            );
            for identity in [
                tool.name.to_owned(),
                format!("agena.{}", tool.name),
                format!("agena_{}", tool.name.replacen('.', "_", 1)),
            ] {
                assert_eq!(cloud_tool(&identity).unwrap().name, tool.name);
                assert_eq!(canonical_cloud_identity(&identity), tool.name);
            }
        }
        for name in [
            "missing.tool",
            "chatgpt.cloud_missing",
            "agena.unknown.tool",
        ] {
            assert!(cloud_tool(name).is_none());
            assert_eq!(canonical_cloud_identity(name), name);
        }
    }
}
