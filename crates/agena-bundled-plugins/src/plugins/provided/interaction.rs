use std::sync::Arc;

use crate::part::{AskUserToolInput, InteractionNotifyToolInput};
use crate::plugins::provided::workflow::{WorkflowPlugin, WorkflowPluginConfig};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput};

pub(crate) const INTERACTION_PLUGIN_ID: &str = "agena.interaction";

pub(crate) struct InteractionPlugin {
    inner: WorkflowPlugin,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "interaction",
    version = env!("CARGO_PKG_VERSION"),
    summary = "User interaction tools.",
    translations(
        locale("zh-CN", summary = "与用户交互的工具。"),
        locale("zh-TW", summary = "與使用者互動的工具。"),
        locale("ja-JP", summary = "ユーザーとのやり取りに使うツールです。"),
        locale("ko-KR", summary = "사용자와 상호작용하는 도구입니다."),
        locale("fr-FR", summary = "Outils d’interaction avec l’utilisateur."),
        locale("de-DE", summary = "Werkzeuge für die Interaktion mit Nutzenden."),
        locale("es-ES", summary = "Herramientas para interactuar con el usuario."),
        locale("hi-IN", summary = "उपयोगकर्ता से बातचीत करने के टूल।"),
        locale("ar-SA", summary = "أدوات للتفاعل مع المستخدم."),
        locale("pt-BR", summary = "Ferramentas de interação com a pessoa usuária.")
    ),
)]
impl InteractionPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(interactive),
        summary = "Ask the user for short structured input.",
        help = "Use for a decision that belongs to the user: a preference, an ambiguous requirement, authorization for a specific dangerous action, or whether to squash a concrete commit range before an authorized push. Show the target, consequences, and distinct options including a safe decline/defer choice. Reuse authorization already given for the same action and scope. If a sensible default exists within that authorization or you can verify the answer yourself, proceed. Ask all necessary questions together through this tool, never by ending the turn with a plain-text question. A timeout, cancellation, or empty answer is not approval; continue only independent, already-authorized work. Do not ask a generic 'should I proceed?' or seek plan approval here; use plan.review for plan approval.",
        translations(
            locale(
                "zh-CN",
                summary = "向用户询问简短、结构化的问题。",
                help = "仅在决定确实属于用户时提问，例如偏好、含糊需求、某项具体高风险操作的授权，或在已获准推送前是否压缩明确的提交范围。说明目标和后果，列出清楚不同的选项，并提供安全的拒绝或暂缓选项。同一操作和范围已获授权时沿用该授权。若授权范围内有合理默认值，或你能自行核实答案，就直接继续。需要询问的内容应一次通过此工具提出，不要在普通回复结尾只留一个问题。超时、取消或空回答都不代表批准；只能继续处理独立且已获授权的工作。不要泛泛询问“要继续吗”，也不要在此征求计划批准；计划批准请使用 plan.review。"
            ),
            locale(
                "zh-TW",
                summary = "向使用者提出簡短、結構化的問題。",
                help = "只有決定確實屬於使用者時才提問，例如偏好、模糊需求、特定高風險操作的授權，或在已獲准推送前是否壓縮明確的提交範圍。說清楚目標與後果，列出彼此明確不同的選項，並提供安全的拒絕或暫緩選項。同一操作與範圍已獲授權時沿用該授權。若授權範圍內有合理預設值，或你能自行核實答案，就直接繼續。所有必要問題一次透過此工具提出，不要只在一般回覆結尾留下問題。逾時、取消或空白回答都不代表同意；只能繼續處理獨立且已獲授權的工作。不要籠統詢問「要繼續嗎」，也不要在此徵求計畫核准；計畫核准請使用 plan.review。"
            ),
            locale(
                "ja-JP",
                summary = "ユーザーに短く構造化された質問をします。",
                help = "ユーザー自身が決めるべきこと（好み、曖昧な要件、特定の危険な操作の許可、許可済みの push 前に明確なコミット範囲を squash するかどうか）に使います。対象と結果を示し、安全に断る・保留する選択肢を含む、意味の異なる選択肢を提示してください。同じ操作と範囲について得た許可は再利用します。その許可の範囲内で妥当な既定値がある場合や、自分で答えを確認できる場合は進めてください。必要な質問はまとめてこのツールで行い、通常の返信を質問だけで終えないでください。タイムアウト、キャンセル、空の回答は許可ではありません。独立していて既に許可された作業だけ続けてください。漠然と「進めてもよいですか」と聞いたり、ここで計画の承認を求めたりしないでください。計画の承認には plan.review を使います。"
            ),
            locale(
                "ko-KR",
                summary = "사용자에게 짧고 구조화된 질문을 합니다.",
                help = "선호, 모호한 요구사항, 특정 위험 작업의 승인, 승인된 푸시 전에 명확한 커밋 범위를 스쿼시할지처럼 사용자가 결정해야 하는 일에 사용하세요. 대상과 결과를 설명하고, 서로 구분되는 선택지와 안전한 거절·보류 선택지를 제시하세요. 같은 작업과 범위에 이미 받은 승인은 그대로 활용하세요. 승인 범위 안에 합리적인 기본값이 있거나 직접 답을 확인할 수 있으면 진행하세요. 필요한 질문은 한 번에 이 도구로 묻고, 일반 답변을 질문만 남긴 채 끝내지 마세요. 시간 초과, 취소, 빈 답변은 승인이 아닙니다. 독립적이며 이미 승인된 작업만 계속하세요. 막연히 “진행할까요?”라고 묻거나 여기서 계획 승인을 요청하지 마세요. 계획 승인은 plan.review를 사용하세요."
            ),
            locale(
                "fr-FR",
                summary = "Poser à l’utilisateur une question courte et structurée.",
                help = "À utiliser pour une décision qui revient à l’utilisateur : préférence, exigence ambiguë, autorisation d’une action risquée précise ou choix de squash d’une plage de commits définie avant un push déjà autorisé. Présentez la cible, les conséquences et des options réellement distinctes, dont un refus ou un report sans risque. Réutilisez l’autorisation déjà donnée pour la même action et le même périmètre. Si une valeur par défaut raisonnable respecte cette autorisation, ou si vous pouvez vérifier la réponse vous-même, poursuivez. Regroupez toutes les questions nécessaires dans cet outil ; ne terminez jamais un tour par une simple question en texte. Un délai dépassé, une annulation ou une réponse vide ne vaut pas autorisation ; ne poursuivez que les travaux indépendants déjà autorisés. Ne demandez pas vaguement « dois-je continuer ? » et ne sollicitez pas ici l’approbation d’un plan : utilisez plan.review pour cela."
            ),
            locale(
                "de-DE",
                summary = "Dem Nutzer eine kurze, strukturierte Frage stellen.",
                help = "Verwenden Sie das Tool für Entscheidungen, die dem Nutzer zustehen: Präferenzen, unklare Anforderungen, die Freigabe einer konkreten riskanten Aktion oder die Frage, ob ein klar abgegrenzter Commit-Bereich vor einem bereits freigegebenen Push zusammengeführt werden soll. Nennen Sie Ziel und Folgen und bieten Sie klar unterschiedliche Optionen einschließlich sicherer Ablehnung oder Zurückstellung an. Eine Freigabe für dieselbe Aktion und denselben Umfang gilt weiterhin. Gibt es innerhalb dieser Freigabe einen sinnvollen Standardwert oder können Sie die Antwort selbst prüfen, fahren Sie fort. Stellen Sie alle nötigen Fragen gemeinsam mit diesem Tool; beenden Sie die Antwort nicht mit einer unstrukturierten Textfrage. Zeitüberschreitung, Abbruch oder eine leere Antwort sind keine Zustimmung. Setzen Sie nur unabhängige, bereits freigegebene Arbeiten fort. Fragen Sie nicht pauschal „Soll ich fortfahren?“ und holen Sie hier keine Planfreigabe ein; dafür ist plan.review vorgesehen."
            ),
            locale(
                "es-ES",
                summary = "Haz al usuario una pregunta breve y estructurada.",
                help = "Úsalo para decisiones que corresponden al usuario: una preferencia, un requisito ambiguo, la autorización de una acción peligrosa concreta o si se debe agrupar un intervalo de commits definido antes de un push ya autorizado. Expón el objetivo y las consecuencias, con opciones claramente distintas e incluyendo rechazar o aplazar de forma segura. Reutiliza la autorización existente para la misma acción y el mismo alcance. Si hay un valor predeterminado razonable dentro de esa autorización o puedes comprobar la respuesta, continúa. Reúne todas las preguntas necesarias en esta herramienta; no termines el turno con una pregunta en texto libre. Un tiempo agotado, una cancelación o una respuesta vacía no son aprobación; continúa solo con trabajo independiente ya autorizado. No preguntes de forma genérica «¿sigo?» ni solicites aquí la aprobación de un plan; para eso usa plan.review."
            ),
            locale(
                "hi-IN",
                summary = "उपयोगकर्ता से छोटा, संरचित प्रश्न पूछें।",
                help = "इसका उपयोग उन्हीं फैसलों के लिए करें जो उपयोगकर्ता के हैं—जैसे पसंद, अस्पष्ट आवश्यकता, किसी खास जोखिमपूर्ण कार्रवाई की अनुमति, या पहले से स्वीकृत पुश से पहले निश्चित commit सीमा को squash करना है या नहीं। लक्ष्य और परिणाम बताएँ; अलग-अलग विकल्प दें, जिनमें सुरक्षित रूप से मना करने या टालने का विकल्प भी हो। उसी कार्रवाई और दायरे के लिए पहले मिली अनुमति का फिर उपयोग करें। उस अनुमति के भीतर उचित डिफ़ॉल्ट हो या आप उत्तर खुद जाँच सकें तो आगे बढ़ें। सभी ज़रूरी प्रश्न एक साथ इसी टूल से पूछें; सामान्य उत्तर के अंत में केवल सवाल न छोड़ें। समय-सीमा समाप्त होना, रद्द होना या खाली जवाब मिलना अनुमति नहीं है; केवल स्वतंत्र और पहले से स्वीकृत काम जारी रखें। सामान्य रूप से “क्या मैं आगे बढ़ूँ?” न पूछें और यहाँ योजना की मंज़ूरी न लें; उसके लिए plan.review का उपयोग करें।"
            ),
            locale(
                "ar-SA",
                summary = "اطرح على المستخدم سؤالًا موجزًا ومنظمًا.",
                help = "استخدمها لقرار يخص المستخدم: تفضيل، أو متطلب ملتبس، أو إذن بإجراء خطر محدد، أو اختيار دمج نطاق واضح من الالتزامات قبل دفع سبق أن أُذن به. وضّح الهدف والنتائج وقدّم خيارات مختلفة، بينها الرفض أو التأجيل بأمان. أعد استخدام الإذن الممنوح للإجراء والنطاق نفسيهما. إذا وُجد خيار افتراضي معقول ضمن ذلك الإذن أو أمكنك التحقق من الإجابة بنفسك، فتابع. اجمع كل الأسئلة اللازمة في هذه الأداة، ولا تنه الرد بسؤال نصي عادي. انتهاء المهلة أو الإلغاء أو الإجابة الفارغة لا تعني الموافقة؛ تابع فقط العمل المستقل المأذون به مسبقًا. لا تسأل سؤالًا عامًا مثل «هل أتابع؟» ولا تطلب اعتماد الخطة هنا؛ استخدم plan.review لاعتماد الخطط."
            ),
            locale(
                "pt-BR",
                summary = "Faça ao usuário uma pergunta curta e estruturada.",
                help = "Use para decisões que cabem ao usuário: uma preferência, um requisito ambíguo, autorização para uma ação perigosa específica ou se um intervalo definido de commits deve ser consolidado antes de um push já autorizado. Explique o alvo e as consequências e ofereça opções distintas, incluindo recusar ou adiar com segurança. Reaproveite a autorização dada para a mesma ação e o mesmo escopo. Se houver um padrão razoável dentro dessa autorização ou você puder verificar a resposta, prossiga. Reúna todas as perguntas necessárias nesta ferramenta; não encerre o turno com uma pergunta em texto livre. Tempo esgotado, cancelamento ou resposta vazia não são aprovação; continue apenas o trabalho independente já autorizado. Não pergunte genericamente “devo continuar?” nem peça aprovação de plano aqui; para isso, use plan.review."
            )
        )
    )]
    async fn ask(&self, input: &AskUserToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_ask_user(input).await
    }

    #[tool(
        tags(interactive),
        summary = "Show a non-blocking Markdown notification to the user.",
        translations(
            locale("zh-CN", summary = "向用户显示一条不会阻塞操作的 Markdown 通知。"),
            locale("zh-TW", summary = "向使用者顯示一則不會阻塞操作的 Markdown 通知。"),
            locale(
                "ja-JP",
                summary = "処理を止めずに Markdown 通知をユーザーへ表示します。"
            ),
            locale(
                "ko-KR",
                summary = "작업을 막지 않는 Markdown 알림을 사용자에게 표시합니다."
            ),
            locale(
                "fr-FR",
                summary = "Afficher une notification Markdown sans bloquer l’utilisateur."
            ),
            locale(
                "de-DE",
                summary = "Dem Nutzer eine nicht blockierende Markdown-Benachrichtigung anzeigen."
            ),
            locale(
                "es-ES",
                summary = "Muestra al usuario una notificación Markdown sin bloquear el trabajo."
            ),
            locale(
                "hi-IN",
                summary = "उपयोगकर्ता को ऐसी Markdown सूचना दिखाएँ जो काम न रोके।"
            ),
            locale("ar-SA", summary = "اعرض للمستخدم إشعار Markdown دون إيقاف العمل."),
            locale(
                "pt-BR",
                summary = "Mostre uma notificação Markdown sem bloquear o trabalho do usuário."
            )
        )
    )]
    fn notify(&self, input: &InteractionNotifyToolInput) -> SdkResult<ToolInvokeOutput> {
        let input = InteractionNotifyToolInput::parse_input(
            serde_json::to_value(input)
                .map_err(|err| agena_plugin_host::sdk::PluginError::invalid_params_error(&err))?,
        )?;
        let level = input.level.as_str();
        let title = if input.title.is_empty() {
            match input.level {
                agena_domain::InteractionNotificationLevel::Info => "Notice",
                agena_domain::InteractionNotificationLevel::Success => "Completed",
                agena_domain::InteractionNotificationLevel::Warning => "Attention",
                agena_domain::InteractionNotificationLevel::Error => "Error",
            }
            .to_string()
        } else {
            input.title
        };
        Ok(ToolInvokeOutput::from_parts(
            title.clone(),
            format!("{level} notification"),
            input.body_markdown.clone(),
            Some(serde_json::json!({
                "title": title,
                "body_markdown": input.body_markdown,
                "level": level,
            })),
            std::collections::BTreeMap::from([
                ("agena.effect".to_string(), "notification".to_string()),
                ("agena.notification.level".to_string(), level.to_string()),
            ]),
            Vec::new(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::part::{AskUserToolInput, InteractionNotifyToolInput};
    use agena_plugin_host::sdk::Plugin;

    use super::InteractionPlugin;

    #[test]
    fn manifest_contains_only_user_interaction_tools() {
        let manifest = InteractionPlugin::new().manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "interaction");
        assert_eq!(tool_names, ["ask", "notify"]);
        // Both tools are interactive: the batch fan-out must not special-case
        // them, so nothing here asserts anything about host-side ordering.
    }

    #[test]
    fn ask_supports_bounded_auto_resolution_without_ids_or_previews() {
        let parsed = AskUserToolInput::parse_input(serde_json::json!({
            "auto_resolution_ms": 60_000,
            "questions": [{
                "question": "Choose",
                "options": [{
                    "label": "A"
                }]
            }]
        }))
        .expect("valid ask input");
        assert_eq!(parsed.auto_resolution_ms, Some(60_000));
        assert_eq!(parsed.questions[0].options[0].label, "A");

        let err = AskUserToolInput::parse_input(serde_json::json!({
            "auto_resolution_ms": 59_999,
            "questions": [{
                "question": "Choose",
                "allow_custom": true
            }]
        }))
        .expect_err("sub-minute auto resolution must be rejected");
        assert!(err.diagnostic_message().contains("auto_resolution_ms"));
        assert!(err.to_string().contains("auto_resolution_ms"));
    }

    #[test]
    fn notification_input_is_trimmed_and_requires_a_body() {
        let parsed = InteractionNotifyToolInput::parse_input(serde_json::json!({
            "title": "  Build complete  ",
            "body_markdown": "  **Done**  ",
            "level": "success"
        }))
        .expect("valid notification input");
        assert_eq!(parsed.title, "Build complete");
        assert_eq!(parsed.body_markdown, "**Done**");
        assert_eq!(parsed.level.as_str(), "success");

        assert!(
            InteractionNotifyToolInput::parse_input(serde_json::json!({
                "body_markdown": "   "
            }))
            .is_err()
        );
    }

    #[test]
    fn notification_output_keeps_markdown_and_tui_severity() {
        let input = InteractionNotifyToolInput::parse_input(serde_json::json!({
            "title": "Release",
            "body_markdown": "## Ready",
            "level": "warning"
        }))
        .expect("valid notification input");
        let output = InteractionPlugin::new()
            .notify(&input)
            .expect("notification output");
        assert_eq!(output.title, "Release");
        assert_eq!(output.output_text, "## Ready");
        assert_eq!(
            output.metadata.get("agena.notification.level"),
            Some(&"warning".to_string())
        );
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload.get("level"))
                .and_then(serde_json::Value::as_str),
            Some("warning")
        );
    }
}
