//! Single-call core guide and versioned inline reply publication.
mod errors;
use agena_domain::{InlineDocument, InlinePhase, PreparedInlineUi, UiKind, ui_supported_operators};
use agena_macros::ToolInput;
use agena_plugin_host::sdk::{
    PluginError, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput,
};
use errors::{invalid_document, invalid_update};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const INLINE_UI_PLUGIN_ID: &str = "agena.inline_ui";
pub(crate) struct InlineUiPlugin {
    store: agena_runtime_plugins::plugins::inline_ui::InlineUiStore,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
enum GuideSection {
    Schema,
    Dashboard,
    Expressions,
    Updates,
    Chart,
    Paragraph,
    Callout,
    Steps,
    Comparison,
    Metrics,
    Table,
    Details,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct GuideInput {
    /// Empty/default: complete core guide and tested 7-node example.
    /// Nonempty: only these supplemental sections in one call.
    #[serde(default)]
    sections: Vec<GuideSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct PrepareInput {
    /// Bounded semantic document: version=1, optional title, ordered blocks.
    /// Document v1; guide() covers the common contract. No executable code.
    document: Value,
    /// Complete by default; working when the agent will publish later updates.
    #[serde(default)]
    phase: Option<InlinePhase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    reference: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct UpdateInput {
    update: agena_domain::InlineUpdate,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena", name = "inline_ui", version = env!("CARGO_PKG_VERSION"),
    summary = "Inline Generative UI: composable, stateful interfaces and live work updates inside answers. guide() teaches the complete core workflow; use sections only for advanced references. prepare/read/update.",
    translations(
        locale("zh-CN", summary = "内嵌式生成 UI：guide() 一次说明核心用法，低频章节可批量查阅；准备后在正文嵌入引用。"),
        locale("zh-TW", summary = "內嵌式生成 UI：guide() 一次說明核心用法，進階章節可批量查閱；在正文嵌入引用。"),
        locale("ja-JP", summary = "回答内のインタラクティブ UI。guide() 一回で基本を学び、追加資料は複数章まとめて取得できます。"),
        locale("ko-KR", summary = "답변 안의 인터랙티브 UI. guide() 한 번으로 핵심을 배우고 추가 자료는 여러 장을 함께 조회합니다."),
        locale("fr-FR", summary = "Interface intégrée aux réponses. guide() enseigne le flux essentiel en un appel ; les annexes sont groupables."),
        locale("de-DE", summary = "Interaktive UI direkt in Antworten. guide() erklärt den Kern in einem Aufruf; Zusatzkapitel sind bündelbar."),
        locale("es-ES", summary = "UI integrada en respuestas. guide() explica lo esencial de una vez; los anexos pueden solicitarse en lote."),
        locale("hi-IN", summary = "उत्तर के भीतर इंटरैक्टिव UI। guide() एक बार में मूल अनुबंध सिखाता है; अतिरिक्त अनुभाग साथ मँगाएँ।"),
        locale("ar-SA", summary = "واجهات تفاعلية داخل الإجابات. يشرح guide() الأساسيات مرة واحدة، وتُطلب الأقسام الإضافية دفعة واحدة."),
        locale("pt-BR", summary = "UI integrada às respostas. guide() explica o essencial de uma vez; seções extras podem vir em lote.")
    )
)]
impl InlineUiPlugin {
    pub(crate) fn new() -> Self {
        Self {
            store: Default::default(),
        }
    }

    #[tool(
        tags(read_only, discovery),
        name = "guide",
        summary = "Learn the complete core panel, state, action and live-update workflow in one call; batch optional advanced sections.",
        help = "guide() is a self-contained first-use manual with a small tested panel and update. guide({sections:[\"chart\",\"expressions\"]}) fetches optional references without repeating the core. Once learned, call prepare directly. The embed must be placed in the answer.",
        translations(
            locale(
                "zh-CN",
                summary = "guide() 一次提供交互面板、状态、动作和更新核心说明；补充章节支持批量读取。"
            ),
            locale(
                "zh-TW",
                summary = "guide() 一次提供面板、狀態、動作和更新核心說明；額外章節可批量讀取。"
            ),
            locale(
                "ja-JP",
                summary = "一回で基本のパネル・状態・更新を学び、補足章はまとめて取得します。"
            ),
            locale(
                "ko-KR",
                summary = "한 번에 패널·상태·업데이트 핵심을 배우고 추가 장은 일괄 조회합니다."
            ),
            locale(
                "fr-FR",
                summary = "Apprendre tout le flux de base en un appel ; obtenir les annexes en lot."
            ),
            locale(
                "de-DE",
                summary = "Kernvertrag in einem Aufruf lernen; Zusatzkapitel gesammelt lesen."
            ),
            locale(
                "es-ES",
                summary = "Aprende el flujo básico en una llamada; consulta anexos en lote."
            ),
            locale("hi-IN", summary = "पूरा मूल प्रवाह एक कॉल में सीखें; अतिरिक्त अनुभाग साथ लें।"),
            locale(
                "ar-SA",
                summary = "تعرّف إلى العقد الأساسي بمكالمة واحدة؛ واحصل على الملاحق دفعة واحدة."
            ),
            locale(
                "pt-BR",
                summary = "Aprenda o fluxo essencial numa chamada; obtenha anexos em lote."
            )
        )
    )]
    async fn guide(&self, input: &GuideInput) -> SdkResult<ToolInvokeOutput> {
        const MAX_SECTIONS: usize = 6;
        const MAX_GUIDE_BYTES: usize = 160 * 1024;
        if input.sections.is_empty() {
            return Ok(ToolInvokeOutput::text(core_guide()?));
        }
        let mut selected = std::collections::BTreeSet::new();
        let mut result = String::new();
        for section in &input.sections {
            if !selected.insert(*section) {
                continue;
            }
            if selected.len() > MAX_SECTIONS {
                return Err(PluginError::invalid_params_with_public_detail(
                    "too many guide sections",
                    "guide.sections: request up to 6 distinct supplemental sections per call.",
                ));
            }
            let text = supplemental_section(*section)?;
            if result.len() + text.len() + 2 > MAX_GUIDE_BYTES {
                return Err(PluginError::invalid_params_with_public_detail(
                    "guide output budget exceeded",
                    "guide.sections: references exceed 160 KiB; request fewer sections. No partial guide was returned.",
                ));
            }
            result.push_str(&text);
            result.push_str("\n\n");
        }
        Ok(ToolInvokeOutput::text(result))
    }

    #[tool(
        tags(read_only),
        name = "prepare",
        summary = "Prepare immutable rich content and return an embed reference. Insert it at the desired position in your normal answer; this tool does not display the content.",
        help = "Learn via guide() if needed; no pre-validation call is required. Prepares one versioned passage without displaying it. Paste the entire returned inline-ui fenced block at its intended position between normal Markdown paragraphs. Keep only the reference ID inside; do not add an outer code fence. All text is plain text; no HTML/JS, services, callbacks, or side effects. Use supplied facts, label examples, and keep sources/uncertainty. Do include the returned embed in the final answer; do not print the document JSON. Invalid input creates no reference.",
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
            .map_err(|error| invalid_document(&input.document, &error))?;
        if context.call_id <= 0 {
            return Err(PluginError::invalid_params(
                "prepare requires a durable tool invocation",
            ));
        }
        let prepared = PreparedInlineUi {
            reference: format!("ui_{}_{}", context.call_id, uuid::Uuid::new_v4().simple()),
            document,
        };
        let store = self.store.clone();
        let session = context.session_id;
        let call = context.call_id;
        let reference = prepared.reference.clone();
        let document = prepared.document.clone();
        let input_phase = input.phase.unwrap_or(InlinePhase::Complete);
        let snapshot = tokio::task::spawn_blocking(move || {
            store.create(session, &reference, document, call, input_phase)
        })
        .await
        .map_err(|error| PluginError::internal_error(&error))?
        .map_err(|error| PluginError::internal_error(&error))?;
        let marker = prepared.marker();
        Ok(ToolInvokeOutput::from_parts(
            "Prepared inline content",
            "Ready to embed in the answer; not displayed here.",
            format!(
                "Insert this entire inline-ui fenced block unchanged between prose paragraphs in your normal answer:\n{marker}\nThis preparation has not displayed the content. Do not print the document JSON or wrap the block in another fence."
            ),
            Some(
                json!({"inline_ui_prepared": prepared, "revision": snapshot.revision, "phase": snapshot.phase}),
            ),
            Default::default(),
            Vec::new(),
        ))
    }
    #[tool(
        tags(read_only),
        name = "read",
        summary = "Inspect the current UI for context recovery or revision-conflict reconciliation; not required before normal sequential updates. User-local selections are not agent data.",
        translations(
            locale("zh-CN", summary = "读取当前界面的版本、数据与结构。"),
            locale("zh-TW", summary = "讀取目前介面的版本、資料與結構。"),
            locale("ja-JP", summary = "現在のUIリビジョンと内容を読み取ります。"),
            locale("ko-KR", summary = "현재 UI 버전과 내용을 읽습니다."),
            locale("fr-FR", summary = "Lire la révision et le contenu de l’interface."),
            locale("de-DE", summary = "Aktuelle UI-Version und Inhalte lesen."),
            locale("es-ES", summary = "Lee la revisión y contenido de la interfaz."),
            locale("hi-IN", summary = "वर्तमान UI संस्करण और सामग्री पढ़ें।"),
            locale("ar-SA", summary = "اقرأ إصدار الواجهة ومحتواها الحالي."),
            locale("pt-BR", summary = "Leia a revisão e o conteúdo da interface.")
        )
    )]
    async fn read(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &ReadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        if agena_domain::inline_reference_tool_id(&input.reference).is_none() {
            return Err(PluginError::invalid_params_with_public_detail(
                "invalid inline reference",
                "read.reference: use the exact reference returned by prepare in this session.",
            ));
        }
        let store = self.store.clone();
        let session = context.session_id;
        let reference = input.reference.clone();
        let snapshot = tokio::task::spawn_blocking(move || store.read(session, &reference))
            .await
            .map_err(|error| PluginError::internal_error(&error))?
            .map_err(|error| PluginError::internal_error(&error))?
            .ok_or_else(|| PluginError::invalid_params("UI not found in the current session"))?;
        Ok(ToolInvokeOutput::text(
            serde_json::to_string(&snapshot)
                .map_err(|error| PluginError::internal_error(&error))?,
        ))
    }

    #[tool(
        tags(mutate),
        name = "update",
        summary = "Atomically update the same inline UI while working: data, nodes, complete document or phase. Requires expected_revision; conflicts apply nothing. Existing user input is preserved.",
        help = "Display the prepared reference early in the answer, then call update after actual work changes. Prefer a panel patch with block_id and changed data or whole-node upserts. Never invent progress. Use phase=complete when finished. This changes only agent-authored UI content, not the user's local selections or external services.",
        translations(
            locale(
                "zh-CN",
                summary = "按预期版本原子更新已显示界面；保留用户的本地输入。"
            ),
            locale("zh-TW", summary = "依預期版本原子更新介面，保留使用者的本地輸入。"),
            locale("ja-JP", summary = "表示中のUIをリビジョン確認付きで更新します。"),
            locale("ko-KR", summary = "버전을 확인하고 표시된 UI를 갱신합니다."),
            locale(
                "fr-FR",
                summary = "Mettre à jour l’interface avec contrôle de révision."
            ),
            locale(
                "de-DE",
                summary = "Die angezeigte Oberfläche revisionsgeprüft aktualisieren."
            ),
            locale("es-ES", summary = "Actualiza la interfaz comprobando su revisión."),
            locale("hi-IN", summary = "संस्करण जाँचकर दिखाई दे रही UI अपडेट करें।"),
            locale("ar-SA", summary = "حدّث الواجهة مع التحقق من الإصدار."),
            locale("pt-BR", summary = "Atualize a interface com controle de revisão.")
        )
    )]
    async fn update(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &UpdateInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let store = self.store.clone();
        let session = context.session_id;
        let call = context.call_id;
        let update = input.update.clone();
        let snapshot = tokio::task::spawn_blocking(move || store.update(session, &update, call))
            .await
            .map_err(|error| PluginError::internal_error(&error))?
            .map_err(|error| invalid_update(&input.update, error))?;
        Ok(ToolInvokeOutput::text(json!({"reference":snapshot.reference,"revision":snapshot.revision,"phase":snapshot.phase}).to_string()))
    }
}

const MAX_CORE_GUIDE_BYTES: usize = 24 * 1024;

fn core_guide() -> SdkResult<String> {
    let catalog = UiKind::all()
        .iter()
        .map(|kind| {
            let name = serde_json::to_value(kind).expect("kind serializes");
            format!(
                "- {}: {}",
                name.as_str().expect("kind string"),
                kind.allowed_props().join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = include_str!("inline_ui/core.md")
        .replace("{COMPONENT_CATALOG}", &catalog)
        .replace("{OPERATORS}", &ui_supported_operators().join(", "))
        .replace(
            "{EXAMPLE}",
            include_str!("../../../../../examples/inline-ui/quick-start.json").trim(),
        )
        .replace(
            "{UPDATE}",
            include_str!("../../../../../examples/inline-ui/quick-start-update.json").trim(),
        );
    if text.len() > MAX_CORE_GUIDE_BYTES {
        return Err(PluginError::internal(
            "core guide exceeds 24 KiB; fix the source rather than truncating it",
        ));
    }
    Ok(text)
}

fn supplemental_section(section: GuideSection) -> SdkResult<String> {
    match section {
        GuideSection::Schema => Ok(format!(
            "# Full document JSON Schema (on demand)\n\n\x60\x60\x60json\n{}\n\x60\x60\x60",
            serde_json::to_string_pretty(&schemars::schema_for!(InlineDocument))
                .map_err(|error| PluginError::internal_error(&error))?
        )),
        GuideSection::Dashboard => Ok(format!(
            "# Large dashboard reference (on demand)\n\n\x60\x60\x60json\n{}\n\x60\x60\x60",
            include_str!("../../../../../examples/inline-ui/live-dashboard.json")
        )),
        GuideSection::Expressions => Ok(format!(
            "# Advanced expressions and evaluation\n\n{}",
            include_str!("inline_ui/state.md")
        )),
        GuideSection::Updates => Ok(format!(
            "# Advanced live updates\n\n{}",
            include_str!("inline_ui/live.md")
        )),
        other => {
            let kind =
                serde_json::to_value(other).map_err(|error| PluginError::internal_error(&error))?;
            let kind = kind.as_str().expect("section kind string");
            let sample: Value =
                serde_json::from_str(include_str!("../../../../../examples/inline-ui/reply.json"))
                    .map_err(|error| PluginError::internal_error(&error))?;
            let block = sample["blocks"]
                .as_array()
                .and_then(|blocks| blocks.iter().find(|b| b["type"] == kind))
                .ok_or_else(|| PluginError::internal("missing static block example"))?;
            Ok(format!(
                "# {kind}: static block schema and reference\n\n\x60\x60\x60json\n{}\n\x60\x60\x60\n\n## Example\n\n\x60\x60\x60json\n{}\n\x60\x60\x60",
                serde_json::to_string_pretty(&topic_schema(kind)?)
                    .map_err(|error| PluginError::internal_error(&error))?,
                serde_json::to_string_pretty(&json!({"version":1,"blocks":[block]}))
                    .map_err(|error| PluginError::internal_error(&error))?
            ))
        }
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
    use agena_domain::{InlineBlock, InlineUpdate, UiState};

    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../../../../examples/inline-ui/quick-start.json"
        ))
        .unwrap()
    }
    fn context(call_id: i64) -> ToolInvokeContext<'static> {
        ToolInvokeContext {
            tool_name: "prepare",
            session_id: 1,
            call_id,
            workspace_root: ".",
            cancellation: Default::default(),
        }
    }
    fn plugin(root: &std::path::Path) -> InlineUiPlugin {
        InlineUiPlugin {
            store: agena_runtime_plugins::plugins::inline_ui::InlineUiStore::new(root),
        }
    }

    #[tokio::test]
    async fn default_core_is_complete_bounded_and_small_without_optional_references() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = plugin(dir.path());
        let core = plugin
            .guide(&GuideInput { sections: vec![] })
            .await
            .unwrap()
            .output_text;
        eprintln!(
            "inline_ui core guide: {} UTF-8 bytes, {} characters",
            core.len(),
            core.chars().count()
        );
        assert!(core.len() <= MAX_CORE_GUIDE_BYTES);
        for key in [
            "prepare",
            "update",
            "expected_revision",
            "revision_conflict",
            "phase",
            "document",
            "blocks",
            "program",
            "root",
            "children",
            "computed",
            "data",
            "state",
            "actions",
            "bind",
            "$get",
            "$op",
            "visible",
            "sum",
            "map",
            "filter",
            "reset",
            "copy",
            "set",
            "toggle",
            "Web/TUI",
            "local",
            "illustrative",
            "REFERENCE_FROM_PREPARE",
        ] {
            assert!(core.contains(key), "core guide missing {key}");
        }
        assert!(core.contains("Agent-supplied baseline"));
        assert!(!core.contains("Full document JSON Schema"));
        assert!(!core.contains("Large dashboard reference"));
        assert!(!core.contains("validate(document"));
        assert!(!core.contains("{COMPONENT_CATALOG}"));
        assert!(!core.contains("{EXAMPLE}"));
        let supplement = plugin
            .guide(&GuideInput {
                sections: vec![
                    GuideSection::Chart,
                    GuideSection::Expressions,
                    GuideSection::Chart,
                ],
            })
            .await
            .unwrap()
            .output_text;
        eprintln!("inline_ui chart+expressions: {} bytes", supplement.len());
        assert_eq!(
            supplement.matches("# chart: static block schema").count(),
            1
        );
        assert!(supplement.contains("filter/map take"));
        assert!(!supplement.contains("Small complete working panel"));
        let schema = plugin
            .guide(&GuideInput {
                sections: vec![GuideSection::Schema],
            })
            .await
            .unwrap()
            .output_text;
        eprintln!(
            "inline_ui supplemental schema: {} UTF-8 bytes",
            schema.len()
        );
        assert!(schema.contains("Full document JSON Schema"));
        assert!(!schema.contains("Agent-supplied baseline"));
        let dashboard = plugin
            .guide(&GuideInput {
                sections: vec![GuideSection::Dashboard],
            })
            .await
            .unwrap()
            .output_text;
        eprintln!(
            "inline_ui supplemental dashboard: {} UTF-8 bytes",
            dashboard.len()
        );
        assert!(dashboard.contains("Large dashboard reference"));
        assert!(dashboard.contains("agentProgress"));
        assert!(!core.contains("agentProgress"));
        assert!(serde_json::from_value::<GuideInput>(json!({"sections":["invented"]})).is_err());
        let too_many = plugin
            .guide(&GuideInput {
                sections: vec![
                    GuideSection::Schema,
                    GuideSection::Dashboard,
                    GuideSection::Expressions,
                    GuideSection::Updates,
                    GuideSection::Chart,
                    GuideSection::Paragraph,
                    GuideSection::Callout,
                ],
            })
            .await
            .unwrap_err();
        assert!(too_many.failure.user.fallback.contains("6 distinct"));
    }

    #[tokio::test]
    async fn prepare_update_and_inline_error_flow_needs_no_extra_validate_or_read() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = plugin(dir.path());
        let input = fixture();
        let original = InlineDocument::from_value(input.clone()).unwrap();
        let InlineBlock::Panel { program, .. } = &original.blocks[0] else {
            panic!("panel")
        };
        assert_eq!(program.nodes.len(), 7);
        let before = program.render(&UiState::new()).unwrap();
        assert_eq!(
            before.root.find("summary").unwrap().props["value"],
            json!(150.0)
        );
        assert!(before.root.find("warning").is_none());
        let changed = program
            .input(&before.state, "slider", json!(50))
            .unwrap()
            .state;
        let view = program.render(&changed).unwrap();
        assert_eq!(
            view.root.find("summary").unwrap().props["value"],
            json!(180.0)
        );
        assert!(view.root.find("warning").is_some());
        assert_eq!(
            program.activate(&changed, "reset").unwrap().state["percent"],
            json!(25)
        );

        let invalid = {
            let mut v = input.clone();
            v["blocks"][0]["program"]["nodes"][2]["props"]["style"] = json!("color:red");
            v
        };
        let error = plugin
            .prepare(
                &context(120),
                &PrepareInput {
                    document: invalid,
                    phase: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.diagnostic.data.as_ref().unwrap()["agena_inline_ui_error"]["code"],
            "invalid_ui"
        );
        assert_eq!(
            error.diagnostic.data.as_ref().unwrap()["agena_inline_ui_error"]["applied"],
            false
        );
        assert_eq!(
            error.diagnostic.data.as_ref().unwrap()["agena_inline_ui_error"]["issues"][0]["node_id"],
            "base"
        );
        assert!(error.failure.user.fallback.contains("props.style"));
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "failed preparation must not create a head"
        );

        // Correcting the reported property is enough; no extra preflight.
        let corrected = plugin
            .prepare(
                &context(121),
                &PrepareInput {
                    document: input.clone(),
                    phase: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(corrected.payload.as_ref().unwrap()["revision"], 1);

        let prepared = plugin
            .prepare(
                &context(123),
                &PrepareInput {
                    document: input.clone(),
                    phase: None,
                },
            )
            .await
            .unwrap();
        let payload = prepared.payload.as_ref().unwrap();
        assert_eq!(payload["phase"], "complete");
        assert_eq!(payload["revision"], 1);
        let reference = payload["inline_ui_prepared"]["reference"]
            .as_str()
            .unwrap()
            .to_owned();
        let first = plugin.store.read(1, &reference).unwrap().unwrap();
        assert_eq!(first.phase, InlinePhase::Complete);
        assert_eq!(first.revision, 1);

        let update_example: Value = serde_json::from_str(include_str!(
            "../../../../../examples/inline-ui/quick-start-update.json"
        ))
        .unwrap();
        let mut update = update_example["update"].clone();
        update["reference"] = json!(reference);
        let update: InlineUpdate = serde_json::from_value(update).unwrap();
        let result = plugin
            .update(
                &context(124),
                &UpdateInput {
                    update: update.clone(),
                },
            )
            .await
            .unwrap();
        let result: Value = serde_json::from_str(&result.output_text).unwrap();
        assert_eq!(result["revision"], 2);
        assert_eq!(result["phase"], "complete");
        let second = plugin.store.read(1, &reference).unwrap().unwrap();
        let InlineBlock::Panel { program: next, .. } = &second.document.blocks[0] else {
            panic!("panel")
        };
        let view = next.render(&changed).unwrap();
        assert_eq!(view.state["percent"], json!(50));
        assert_eq!(
            view.root.find("summary").unwrap().props["value"],
            json!(240.0)
        );

        let conflict = plugin
            .update(&context(125), &UpdateInput { update })
            .await
            .unwrap_err();
        let details = &conflict.diagnostic.data.as_ref().unwrap()["agena_inline_ui_error"];
        assert_eq!(details["code"], "revision_conflict");
        assert_eq!(details["expected_revision"], 1);
        assert_eq!(details["current_revision"], 2);
        assert_eq!(details["applied"], false);
        assert_eq!(plugin.store.read(1, &reference).unwrap().unwrap(), second);

        // Invalid new agent data cannot advance or corrupt the valid head.
        let invalid_update: InlineUpdate = serde_json::from_value(json!({
            "reference":reference, "expected_revision":2,
            "patch":{"block_id":"scenario","data":{"base":"bad-number"}}
        }))
        .unwrap();
        let failure = plugin
            .update(
                &context(127),
                &UpdateInput {
                    update: invalid_update,
                },
            )
            .await
            .unwrap_err();
        let data = &failure.diagnostic.data.as_ref().unwrap()["agena_inline_ui_error"];
        assert_eq!(data["code"], "invalid_ui");
        assert_eq!(data["applied"], false);
        assert_eq!(data["issues"][0]["block_id"], "scenario");
        assert_eq!(data["issues"][0]["path"], "update.patch.data");
        assert_eq!(plugin.store.read(1, &reference).unwrap().unwrap(), second);

        // Whole-node upserts use patch order, not the candidate document's
        // flat node index. A misindexed repair hint can corrupt a retry.
        let bad_node: InlineUpdate = serde_json::from_value(json!({
            "reference":reference, "expected_revision":2,
            "patch":{"block_id":"scenario","nodes":[{
                "id":"summary","kind":"metric",
                "props":{"label":"Adjusted total","value":0,"style":"unsupported"}
            }]}
        }))
        .unwrap();
        let bad_node_error = plugin
            .update(&context(131), &UpdateInput { update: bad_node })
            .await
            .unwrap_err();
        let bad_details =
            &bad_node_error.diagnostic.data.as_ref().unwrap()["agena_inline_ui_error"];
        assert_eq!(bad_details["applied"], false);
        assert_eq!(
            bad_details["issues"][0]["path"],
            "update.patch.nodes[0].props.style"
        );
        assert_eq!(bad_details["issues"][0]["node_id"], "summary");
        assert_eq!(plugin.store.read(1, &reference).unwrap().unwrap(), second);

        // Conflict recovery reads the current head and deliberately merges a
        // separate new observation instead of blindly changing the version.
        let recovered = plugin
            .read(
                &context(128),
                &ReadInput {
                    reference: reference.clone(),
                },
            )
            .await
            .unwrap();
        let head: Value = serde_json::from_str(&recovered.output_text).unwrap();
        assert_eq!(head["revision"], 2);
        assert_eq!(
            head["document"]["blocks"][0]["program"]["data"]["base"],
            160
        );
        let reconciled: InlineUpdate = serde_json::from_value(json!({
            "reference":reference, "expected_revision":2,
            "patch":{"block_id":"scenario","data":{"base":180}}
        }))
        .unwrap();
        let after_reconcile = plugin
            .update(&context(129), &UpdateInput { update: reconciled })
            .await
            .unwrap();
        let after_reconcile: Value = serde_json::from_str(&after_reconcile.output_text).unwrap();
        assert_eq!(after_reconcile["revision"], 3);
        assert_eq!(
            plugin.store.read(1, &reference).unwrap().unwrap().revision,
            3
        );

        let working = plugin
            .prepare(
                &context(126),
                &PrepareInput {
                    document: input,
                    phase: Some(InlinePhase::Working),
                },
            )
            .await
            .unwrap();
        assert_eq!(working.payload.as_ref().unwrap()["phase"], "working");
        let active_ref = working.payload.as_ref().unwrap()["inline_ui_prepared"]["reference"]
            .as_str()
            .unwrap();
        assert_eq!(
            plugin.store.read(1, active_ref).unwrap().unwrap().phase,
            InlinePhase::Working
        );
        let finish: InlineUpdate = serde_json::from_value(json!({
            "reference":active_ref,"expected_revision":1,"phase":"complete"
        }))
        .unwrap();
        let done = plugin
            .update(&context(130), &UpdateInput { update: finish })
            .await
            .unwrap();
        let done: Value = serde_json::from_str(&done.output_text).unwrap();
        assert_eq!(done["revision"], 2);
        assert_eq!(done["phase"], "complete");
        assert_eq!(
            plugin.store.read(1, active_ref).unwrap().unwrap().phase,
            InlinePhase::Complete
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
