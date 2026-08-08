//! Structured findings emitted by review, security and verification workflows.

use agena_macros::ToolInput;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeOutput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const REPORT_PLUGIN_ID: &str = "agena.report";

pub(crate) struct ReportPlugin;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FindingSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl std::fmt::Display for FindingSeverity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, ToolInput)]
#[input(
    trim("file", "title", "body"),
    non_empty("file", "title", "body"),
    minimum("line", 1),
    minimum("end_line", 1),
    minimum("confidence", 0),
    maximum("confidence", 1)
)]
#[serde(deny_unknown_fields)]
struct Finding {
    severity: FindingSeverity,
    file: String,
    line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end_line: Option<u32>,
    title: String,
    body: String,
    #[serde(default = "default_confidence")]
    confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<String>,
}

const fn default_confidence() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, ToolInput)]
#[input(
    trim("summary", "findings[].file", "findings[].title", "findings[].body"),
    max_items("findings", 200),
    non_empty("findings[].file", "findings[].title", "findings[].body"),
    minimum("findings[].line", 1),
    minimum("findings[].end_line", 1),
    minimum("findings[].confidence", 0),
    maximum("findings[].confidence", 1)
)]
#[serde(deny_unknown_fields)]
struct ReportFindingsInput {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    findings: Vec<Finding>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "report",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Structured review and verification findings.",
)]
impl ReportPlugin {
    pub(crate) fn new() -> Self {
        Self
    }

    #[tool(
        tags(mutate, discovery),
        name = "findings",
        summary = "Publish structured file-and-line findings for UI and integrations.",
        read_only,
        concurrency_safe
    )]
    async fn invoke_findings(&self, input: &ReportFindingsInput) -> SdkResult<ToolInvokeOutput> {
        let mut lines = Vec::new();
        if !input.summary.is_empty() {
            lines.push(input.summary.clone());
        }
        if input.findings.is_empty() {
            lines.push("No findings.".to_string());
        } else {
            for finding in &input.findings {
                let location = finding
                    .end_line
                    .filter(|end| *end != finding.line)
                    .map_or_else(
                        || format!("{}:{}", finding.file, finding.line),
                        |end| format!("{}:{}-{end}", finding.file, finding.line),
                    );
                lines.push(format!(
                    "- [{}] {} — {} (confidence {:.2})\n  {}",
                    finding.severity, location, finding.title, finding.confidence, finding.body
                ));
            }
        }
        let counts = [
            FindingSeverity::Critical,
            FindingSeverity::High,
            FindingSeverity::Medium,
            FindingSeverity::Low,
            FindingSeverity::Info,
        ]
        .into_iter()
        .map(|severity| {
            (
                severity.to_string(),
                input
                    .findings
                    .iter()
                    .filter(|finding| finding.severity == severity)
                    .count(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
        Ok(ToolInvokeOutput::from_parts(
            format!("{} finding(s)", input.findings.len()),
            input.summary.clone(),
            lines.join("\n\n"),
            Some(serde_json::json!({
                "summary": input.summary,
                "findings": input.findings,
                "counts": counts,
            })),
            std::collections::BTreeMap::from([
                (
                    "finding_count".to_string(),
                    input.findings.len().to_string(),
                ),
                ("agena.effect".to_string(), "report_findings".to_string()),
            ]),
            Vec::new(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::Plugin;

    use super::ReportPlugin;

    #[test]
    fn manifest_exposes_one_structured_findings_tool() {
        let manifest = ReportPlugin::new().manifest();
        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "report");
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.tools[0].name, "findings");
        let schema = manifest.tools[0].input_schema();
        assert!(
            serde_json::to_string(&schema)
                .expect("serialize schema")
                .contains("severity")
        );
        assert_eq!(
            schema.pointer("/properties/findings/maxItems"),
            Some(&serde_json::json!(200))
        );
    }
}
