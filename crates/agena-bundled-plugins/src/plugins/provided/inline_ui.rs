//! Progressive discovery and publication of inert, semantic inline reply content.
use agena_domain::{INLINE_UI_TOOL, InlineDocument};
use agena_macros::ToolInput;
use agena_plugin_host::sdk::{PluginError, Result as SdkResult, ToolInvokeOutput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const INLINE_UI_PLUGIN_ID: &str = "agena.inline_ui";
pub(crate) struct InlineUiPlugin;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum GuideTopic {
    #[default]
    Overview,
    Paragraph,
    Callout,
    Steps,
    Comparison,
    Metrics,
    Table,
    Details,
    Chart,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct GuideInput {
    /// Start with overview; request only the content kind needed for this answer.
    #[serde(default)]
    topic: GuideTopic,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct PresentInput {
    /// Bounded semantic document: version=1, optional title, ordered blocks.
    /// Read guide(topic) for exact schemas. No HTML, commands or executable code.
    document: Value,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena", name = "inline_ui", version = env!("CARGO_PKG_VERSION"),
    summary = "Inline Generative UI: readable steps, comparisons, callouts, metrics, tables and charts inside answers. Start with guide, then query only the needed kind and present.",
    translations(
        locale("zh-CN", summary = "内嵌式生成 UI：在回复中表达步骤、对比、重点与数据。先读 guide，再按需查询类型并 present。"),
        locale("zh-TW", summary = "內嵌式生成 UI：在回覆中表達步驟、比較、重點與資料。先讀 guide，再按需查詢並 present。"),
        locale("ja-JP", summary = "回答内の手順・比較・注記・データを読みやすく表示。guide で必要な種類だけ確認します。"),
        locale("ko-KR", summary = "답변 안의 단계, 비교, 핵심과 데이터를 표현합니다. guide에서 필요한 종류만 확인하세요."),
        locale("fr-FR", summary = "Contenu visuel intégré aux réponses. Consultez guide, puis seulement le type nécessaire."),
        locale("de-DE", summary = "Strukturierte Inhalte direkt in Antworten. Zuerst guide, danach nur den benötigten Typ abfragen."),
        locale("es-ES", summary = "Contenido visual integrado en respuestas. Consulta guide y después solo el tipo necesario."),
        locale("hi-IN", summary = "उत्तर के भीतर चरण, तुलना और डेटा। पहले guide, फिर केवल आवश्यक प्रकार देखें।"),
        locale("ar-SA", summary = "محتوى مرئي داخل الإجابات. ابدأ بـ guide ثم اطلب النوع المطلوب فقط."),
        locale("pt-BR", summary = "Conteúdo visual integrado às respostas. Consulte guide e depois apenas o tipo necessário.")
    )
)]
impl InlineUiPlugin {
    pub(crate) fn new() -> Self {
        Self
    }

    #[tool(
        tags(read_only, discovery),
        name = "guide",
        summary = "Learn Inline Generative UI progressively: overview or one kind's schema and example.",
        help = "Read overview once, then only the needed topic. Results are instructions, not a published UI. Use present for the actual reply; validate is an optional dry run.",
        translations(
            locale(
                "zh-CN",
                summary = "渐进了解内嵌式生成 UI：查看概览，或一个内容类型的结构与示例。"
            ),
            locale(
                "zh-TW",
                summary = "漸進了解內嵌式生成 UI：查看概覽，或單一類型的結構與範例。"
            ),
            locale("ja-JP", summary = "概要、または必要な種類の構造と例を確認します。"),
            locale("ko-KR", summary = "개요 또는 필요한 종류의 구조와 예제를 확인합니다."),
            locale(
                "fr-FR",
                summary = "Consulter la présentation ou le schéma et l’exemple d’un type."
            ),
            locale(
                "de-DE",
                summary = "Übersicht oder Schema und Beispiel eines Inhaltstyps lesen."
            ),
            locale(
                "es-ES",
                summary = "Consulta el resumen o el esquema y ejemplo de un tipo."
            ),
            locale("hi-IN", summary = "अवलोकन या किसी एक प्रकार की संरचना और उदाहरण देखें।"),
            locale("ar-SA", summary = "اعرض نظرة عامة أو مخطط نوع واحد ومثاله."),
            locale(
                "pt-BR",
                summary = "Consulte a visão geral ou o esquema e exemplo de um tipo."
            )
        )
    )]
    async fn guide(&self, input: &GuideInput) -> SdkResult<ToolInvokeOutput> {
        if matches!(input.topic, GuideTopic::Overview) {
            return Ok(ToolInvokeOutput::text(include_str!("inline_ui/guide.md")));
        }
        let kind = serde_json::to_value(input.topic)
            .map_err(|error| PluginError::internal_error(&error))?;
        let kind = kind.as_str().unwrap_or("overview");
        let sample: Value =
            serde_json::from_str(include_str!("../../../../../examples/inline-ui/reply.json"))
                .map_err(|error| PluginError::internal_error(&error))?;
        let block = sample["blocks"]
            .as_array()
            .and_then(|blocks| blocks.iter().find(|b| b["type"] == kind))
            .ok_or_else(|| PluginError::internal("missing inline UI example"))?;
        let example = json!({"version":1,"blocks":[block]});
        let schema = topic_schema(kind)?;
        Ok(ToolInvokeOutput::from_parts(
            format!("Inline Generative UI · {kind}"),
            format!("{kind}: schema and complete example"),
            format!(
                "# Inline Generative UI: {kind}\n\nPublish with {INLINE_UI_TOOL}(document=...). Text is plain text. Use ordinary prose around this passage; do not repeat it or print its JSON. Each document is immutable.\n\n## Schema\n```json\n{}\n```\n\n## Complete example (illustrative content, not measured facts)\n```json\n{}\n```\n\nLimits: 60 KiB per document, 1–24 blocks, unique ids. Use validate for an optional dry run. Chart values must match labels; null means a gap, pie needs one nonnegative series with a positive total.",
                serde_json::to_string_pretty(&schema)
                    .map_err(|error| PluginError::internal_error(&error))?,
                serde_json::to_string_pretty(&example)
                    .map_err(|error| PluginError::internal_error(&error))?
            ),
            None,
            Default::default(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(read_only),
        name = "validate",
        summary = "Check an inline reply document without publishing it; reports precise validation errors.",
        translations(
            locale("zh-CN", summary = "校验富内容回复但不发布，返回具体字段错误。"),
            locale("zh-TW", summary = "驗證富內容回覆但不發布，回傳具體欄位錯誤。"),
            locale("ja-JP", summary = "公開せずに内容を検証します。"),
            locale("ko-KR", summary = "게시하지 않고 내용을 검증합니다."),
            locale("fr-FR", summary = "Valider sans publier."),
            locale("de-DE", summary = "Prüfen, ohne zu veröffentlichen."),
            locale("es-ES", summary = "Valida sin publicar."),
            locale("hi-IN", summary = "प्रकाशित किए बिना सत्यापित करें।"),
            locale("ar-SA", summary = "تحقق من المحتوى دون نشره."),
            locale("pt-BR", summary = "Valide sem publicar.")
        )
    )]
    async fn validate(&self, input: &PresentInput) -> SdkResult<ToolInvokeOutput> {
        let document = InlineDocument::from_value(input.document.clone())
            .map_err(PluginError::invalid_params)?;
        Ok(ToolInvokeOutput::text(format!(
            "Valid Inline Generative UI document ({} blocks). Not published; use present to include it in your answer.",
            document.blocks.len()
        )))
    }

    #[tool(
        tags(read_only),
        name = "present",
        summary = "Include a semantic rich passage directly in the answer, readable in Web and TUI. Not an app.",
        help = "Read guide for the needed kinds. Publishes one immutable passage in conversation order. All text is plain text; no HTML/JS, services, callbacks, or side effects. Use supplied facts, label examples, and keep sources/uncertainty. Do not repeat the passage in final prose. Validation failures publish nothing.",
        translations(
            locale(
                "zh-CN",
                summary = "在回答中直接呈现富内容段落，Web 与 TUI 均可读；不是创建应用。"
            ),
            locale(
                "zh-TW",
                summary = "在回答中直接呈現富內容段落，Web 與 TUI 均可讀；不是建立應用程式。"
            ),
            locale(
                "ja-JP",
                summary = "Web と TUI の回答内に構造化された説明を表示します。"
            ),
            locale("ko-KR", summary = "Web와 TUI 답변 안에 구조화된 설명을 표시합니다."),
            locale(
                "fr-FR",
                summary = "Insérer un passage visuel dans la réponse Web et TUI."
            ),
            locale(
                "de-DE",
                summary = "Strukturierten Inhalt direkt in Web- und TUI-Antworten anzeigen."
            ),
            locale(
                "es-ES",
                summary = "Incluye un pasaje visual en la respuesta Web y TUI."
            ),
            locale("hi-IN", summary = "Web और TUI उत्तर में संरचित सामग्री दिखाएँ।"),
            locale(
                "ar-SA",
                summary = "أدرج محتوى منظماً داخل الإجابة في الويب والواجهة الطرفية."
            ),
            locale("pt-BR", summary = "Inclua conteúdo visual na resposta Web e TUI.")
        )
    )]
    async fn present(&self, input: &PresentInput) -> SdkResult<ToolInvokeOutput> {
        let document = InlineDocument::from_value(input.document.clone())
            .map_err(PluginError::invalid_params)?;
        let text = document.to_markdown();
        Ok(ToolInvokeOutput::from_parts(
            if document.title.is_empty() {
                "Inline Generative UI"
            } else {
                &document.title
            },
            format!("{} rich content blocks", document.blocks.len()),
            text,
            Some(json!({"inline_ui": document})),
            std::collections::BTreeMap::from([("agena.effect".into(), "inline_reply".into())]),
            Vec::new(),
        ))
    }
}

/// Narrow the generated schema to the requested kind and its reachable definitions.
/// Discovery and publication share Rust types, not separately maintained schemas.
fn topic_schema(kind: &str) -> SdkResult<Value> {
    let mut schema = serde_json::to_value(schemars::schema_for!(InlineDocument))
        .map_err(|error| PluginError::internal_error(&error))?;
    let variants = schema.pointer_mut("/$defs/InlineBlock/oneOf");
    if let Some(Value::Array(variants)) = variants {
        variants
            .retain(|v| v.pointer("/properties/type/const").and_then(Value::as_str) == Some(kind));
    } else if let Some(Value::Array(variants)) = schema.pointer_mut("/$defs/InlineBlock/anyOf") {
        variants
            .retain(|v| v.pointer("/properties/type/const").and_then(Value::as_str) == Some(kind));
    }
    let all = schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    schema
        .as_object_mut()
        .expect("schema object")
        .remove("$defs");
    let mut needed = std::collections::BTreeSet::new();
    fn visit(value: &Value, found: &mut std::collections::BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                if let Some(name) = map
                    .get("$ref")
                    .and_then(Value::as_str)
                    .and_then(|s| s.strip_prefix("#/$defs/"))
                {
                    found.insert(name.to_owned());
                }
                for v in map.values() {
                    visit(v, found);
                }
            }
            Value::Array(items) => {
                for v in items {
                    visit(v, found);
                }
            }
            _ => {}
        }
    }
    visit(&schema, &mut needed);
    loop {
        let previous = needed.clone();
        for name in &previous {
            if let Some(definition) = all.get(name) {
                visit(definition, &mut needed);
            }
        }
        if previous == needed {
            break;
        }
    }
    schema["$defs"] = Value::Object(
        all.into_iter()
            .filter(|(name, _)| needed.contains(name))
            .collect(),
    );
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn progressive_guides_and_present_use_the_same_validated_contract() {
        let plugin = InlineUiPlugin::new();
        let value: Value =
            serde_json::from_str(include_str!("../../../../../examples/inline-ui/reply.json"))
                .unwrap();
        for kind in [
            GuideTopic::Paragraph,
            GuideTopic::Callout,
            GuideTopic::Steps,
            GuideTopic::Comparison,
            GuideTopic::Metrics,
            GuideTopic::Table,
            GuideTopic::Details,
            GuideTopic::Chart,
        ] {
            let result = plugin.guide(&GuideInput { topic: kind }).await.unwrap();
            assert!(result.output_text.contains("Complete example"));
            assert!(result.payload.is_none());
        }
        let result = plugin
            .present(&PresentInput {
                document: value.clone(),
            })
            .await
            .unwrap();
        assert!(result.payload.unwrap().get("inline_ui").is_some());
        assert!(result.attachments.is_empty());
        let validation = plugin
            .validate(&PresentInput { document: value })
            .await
            .unwrap();
        assert!(validation.payload.is_none());
        assert!(
            plugin
                .present(&PresentInput {
                    document: json!({"version":1,"blocks":[]})
                })
                .await
                .is_err()
        );
    }
    #[test]
    fn topic_schema_does_not_advertise_unrequested_kinds() {
        let schema = topic_schema("steps").unwrap();
        let text = schema.to_string();
        assert!(text.contains("InlineStep"));
        assert!(!text.contains("InlineSeries"));
        let sample: Value =
            serde_json::from_str(include_str!("../../../../../examples/inline-ui/reply.json"))
                .unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(validator.is_valid(&json!({"version":1,"blocks":[sample["blocks"][1]]})));
        assert!(!validator.is_valid(&json!({"version":1,"blocks":[sample["blocks"][2]]})));
    }
}
