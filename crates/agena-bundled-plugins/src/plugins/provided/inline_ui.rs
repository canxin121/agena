//! Progressive discovery and publication of inert, semantic inline reply content.
use agena_domain::{INLINE_UI_TOOL, InlineDocument, PreparedInlineUi};
use agena_macros::ToolInput;
use agena_plugin_host::sdk::{
    PluginError, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput,
};
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
struct PrepareInput {
    /// Bounded semantic document: version=1, optional title, ordered blocks.
    /// Read guide(topic) for exact schemas. No HTML, commands or executable code.
    document: Value,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena", name = "inline_ui", version = env!("CARGO_PKG_VERSION"),
    summary = "Inline Generative UI: readable steps, comparisons, callouts, metrics, tables and charts inside answers. Start with guide, then query only the needed kind and prepare.",
    translations(
        locale("zh-CN", summary = "内嵌式生成 UI：在回复中表达步骤、对比、重点与数据。先读 guide，再按需查询类型并 prepare。"),
        locale("zh-TW", summary = "內嵌式生成 UI：在回覆中表達步驟、比較、重點與資料。先讀 guide，再按需查詢並 prepare。"),
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
        help = "Read overview once, then only the needed topic. Results are instructions, not a published UI. Prepare content, then put the returned embed in your answer; validate is an optional dry run.",
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
                "# Inline Generative UI: {kind}\n\nPrepare with {INLINE_UI_TOOL}(document=...). This tool does NOT display content. Paste its entire returned inline-ui fenced block unchanged between paragraphs in your normal Markdown answer. The fence contains only the reference ID. Do not wrap it in an additional code fence. Do not print the document JSON. Text fields are plain text and each prepared document is immutable.\n\n## Schema\n```json\n{}\n```\n\n## Complete example (illustrative content, not measured facts)\n```json\n{}\n```\n\nLimits: 60 KiB per document, 1–24 blocks, unique ids. Use validate for an optional dry run. Chart values must match labels; null means a gap, pie needs one nonnegative series with a positive total.",
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
    async fn validate(&self, input: &PrepareInput) -> SdkResult<ToolInvokeOutput> {
        let document = InlineDocument::from_value(input.document.clone())
            .map_err(PluginError::invalid_params)?;
        Ok(ToolInvokeOutput::text(format!(
            "Valid Inline Generative UI document ({} blocks). Not displayed; use prepare to obtain an embed reference, then insert it in the answer.",
            document.blocks.len()
        )))
    }

    #[tool(
        tags(read_only),
        name = "prepare",
        summary = "Prepare immutable rich content and return an embed reference. Insert it at the desired position in your normal answer; this tool does not display the content.",
        help = "Read guide for the needed kinds. Prepares one immutable passage without displaying it. Paste the entire returned inline-ui fenced block at its intended position between normal Markdown paragraphs. Keep only the reference ID inside; do not add an outer code fence. All text is plain text; no HTML/JS, services, callbacks, or side effects. Use supplied facts, label examples, and keep sources/uncertainty. Do include the returned embed in the final answer; do not print the document JSON. Invalid input creates no reference.",
        translations(
            locale(
                "zh-CN",
                summary = "准备富内容并返回引用；将引用插入正常回答的所需位置。工具本身不展示内容。"
            ),
            locale(
                "zh-TW",
                summary = "準備富內容並回傳引用；將引用放入正常回答的指定位置。工具本身不展示內容。"
            ),
            locale(
                "ja-JP",
                summary = "Web と TUI の向けの内容を準備して参照を返します。参照を回答本文に挿入してください。"
            ),
            locale(
                "ko-KR",
                summary = "내용을 준비하고 참조를 반환합니다. 참조를 답변 본문에 삽입하세요."
            ),
            locale(
                "fr-FR",
                summary = "Préparer un contenu et retourner une référence à insérer dans la réponse."
            ),
            locale(
                "de-DE",
                summary = "Inhalt vorbereiten und eine Referenz zum Einfügen in die Antwort zurückgeben."
            ),
            locale(
                "es-ES",
                summary = "Prepara contenido y devuelve una referencia para insertarla en la respuesta."
            ),
            locale("hi-IN", summary = "सामग्री तैयार करें और उत्तर में डालने के लिए संदर्भ लौटाएँ।"),
            locale("ar-SA", summary = "جهّز المحتوى وأعد مرجعاً لإدراجه في نص الإجابة."),
            locale(
                "pt-BR",
                summary = "Prepare conteúdo e retorne uma referência para inserir na resposta."
            )
        )
    )]
    async fn prepare(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &PrepareInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let document = InlineDocument::from_value(input.document.clone())
            .map_err(PluginError::invalid_params)?;
        if context.call_id <= 0 {
            return Err(PluginError::invalid_params(
                "prepare requires a durable tool invocation",
            ));
        }
        let prepared = PreparedInlineUi {
            reference: format!("ui_{}_{}", context.call_id, uuid::Uuid::new_v4().simple()),
            document,
        };
        let marker = prepared.marker();
        Ok(ToolInvokeOutput::from_parts(
            "Prepared inline content",
            "Ready to embed in the answer; not displayed here.",
            format!(
                "Insert this entire inline-ui fenced block unchanged between prose paragraphs in your normal answer:\n{marker}\nThis preparation has not displayed the content. Do not print the document JSON or wrap the block in another fence."
            ),
            Some(json!({"inline_ui_prepared": prepared})),
            Default::default(),
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
    async fn progressive_guides_and_prepare_use_the_same_validated_contract() {
        let plugin = InlineUiPlugin::new();
        let context = ToolInvokeContext {
            tool_name: "prepare",
            session_id: 1,
            call_id: 123,
            workspace_root: ".",
            cancellation: Default::default(),
        };
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
            .prepare(
                &context,
                &PrepareInput {
                    document: value.clone(),
                },
            )
            .await
            .unwrap();
        assert!(result.payload.unwrap().get("inline_ui_prepared").is_some());
        assert!(result.attachments.is_empty());
        let validation = plugin
            .validate(&PrepareInput { document: value })
            .await
            .unwrap();
        assert!(validation.payload.is_none());
        assert!(
            plugin
                .prepare(
                    &context,
                    &PrepareInput {
                        document: json!({"version":1,"blocks":[]})
                    }
                )
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
