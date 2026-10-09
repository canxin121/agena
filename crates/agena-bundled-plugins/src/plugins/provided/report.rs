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
    translations(
        locale("zh-CN", summary = "以结构化形式提交审查与验证发现。"),
        locale("zh-TW", summary = "以結構化格式提交審查與驗證發現。"),
        locale("ja-JP", summary = "レビューや検証で見つかった事項を構造化して報告します。"),
        locale("ko-KR", summary = "검토 및 검증에서 발견한 내용을 구조화해 보고합니다."),
        locale("fr-FR", summary = "Présenter de façon structurée les constats de revue et de vérification."),
        locale("de-DE", summary = "Prüfungs- und Verifikationsergebnisse strukturiert festhalten."),
        locale("es-ES", summary = "Presenta de forma estructurada los hallazgos de revisión y verificación."),
        locale("hi-IN", summary = "समीक्षा और सत्यापन में मिली बातों को संरचित रूप में दर्ज करें।"),
        locale("ar-SA", summary = "قدّم نتائج المراجعة والتحقق بصيغة منظمة."),
        locale("pt-BR", summary = "Registre de forma estruturada os achados de revisão e verificação.")
    ),
)]
impl ReportPlugin {
    pub(crate) fn new() -> Self {
        Self
    }

    #[tool(
        tags(mutate, discovery, read_only),
        name = "findings",
        summary = "Publish structured file-and-line findings for UI and integrations.",
        translations(
            locale(
                "zh-CN",
                summary = "以结构化格式提交带文件和行号的发现，供界面和集成使用。"
            ),
            locale(
                "zh-TW",
                summary = "以結構化格式提交附有檔案與行號的發現，供介面與整合使用。"
            ),
            locale(
                "ja-JP",
                summary = "ファイル名と行番号を含む検出事項を構造化して、画面や連携機能に渡します。"
            ),
            locale(
                "ko-KR",
                summary = "파일과 줄 번호가 포함된 발견 사항을 구조화해 UI와 연동 기능에 전달합니다."
            ),
            locale(
                "fr-FR",
                summary = "Transmettre aux interfaces et intégrations des constats structurés avec fichier et ligne."
            ),
            locale(
                "de-DE",
                summary = "Strukturierte Befunde mit Datei und Zeile an die Oberfläche und Integrationen übergeben."
            ),
            locale(
                "es-ES",
                summary = "Envía hallazgos estructurados con archivo y línea a la interfaz y a las integraciones."
            ),
            locale(
                "hi-IN",
                summary = "UI और इंटीग्रेशन के लिए फ़ाइल और पंक्ति सहित निष्कर्ष संरचित रूप में भेजें।"
            ),
            locale(
                "ar-SA",
                summary = "أرسل نتائج منظمة تتضمن الملف ورقم السطر إلى الواجهة والتكاملات."
            ),
            locale(
                "pt-BR",
                summary = "Envie achados estruturados com arquivo e linha para a interface e as integrações."
            )
        )
    )]
    async fn invoke_findings(&self, input: &ReportFindingsInput) -> SdkResult<ToolInvokeOutput> {
        // An empty list is a valid "no findings" report. Validate members
        // individually rather than requiring wildcard matches to exist.
        for finding in &input.findings {
            if finding.file.trim().is_empty()
                || finding.title.trim().is_empty()
                || finding.body.trim().is_empty()
            {
                return Err(agena_plugin_host::PluginError::invalid_params(
                    "each finding requires a nonempty file, title, and body",
                ));
            }
            if finding.line == 0
                || finding.end_line == Some(0)
                || !finding.confidence.is_finite()
                || !(0.0..=1.0).contains(&finding.confidence)
            {
                return Err(agena_plugin_host::PluginError::invalid_params(
                    "finding lines must be positive and confidence must be between 0 and 1",
                ));
            }
            if finding.end_line.is_some_and(|end| end < finding.line) {
                return Err(agena_plugin_host::PluginError::invalid_params(
                    "finding end_line must not precede line",
                ));
            }
        }
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
            if input.summary.trim().is_empty() {
                format!("{} finding(s)", input.findings.len())
            } else {
                input.summary.clone()
            },
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
