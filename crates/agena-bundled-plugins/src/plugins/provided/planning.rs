#[cfg(test)]
mod revision_tests;
use std::sync::Arc;

use crate::plugins::provided::workflow::{
    PlanEditInput, PlanGetInput, PlanPhaseInput, PlanReviewInput, PlanSetInput, WorkflowPlanConfig,
    WorkflowPlugin, WorkflowPluginConfig,
};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    CommandBeforeInput, CommandBeforeResponse, InitContext, InitOutcome, Result as SdkResult,
    ToolBeforeInput, ToolBeforePatch, ToolInvokeOutput,
};

pub(crate) const PLAN_PLUGIN_ID: &str = "agena.plan";

pub(crate) struct PlanPlugin {
    inner: WorkflowPlugin,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "plan",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Plan orchestration and plan-autorun tools.",
    translations(
        locale("zh-CN", summary = "编排计划并管理计划自动执行。"),
        locale("zh-TW", summary = "編排計畫並管理計畫自動執行。"),
        locale("ja-JP", summary = "計画の作成・管理と、計画に沿った自動実行を扱います。"),
        locale("ko-KR", summary = "계획을 구성하고 계획에 따른 자동 실행을 관리합니다."),
        locale("fr-FR", summary = "Organiser les plans et leur exécution automatisée."),
        locale("de-DE", summary = "Pläne koordinieren und ihre automatische Ausführung verwalten."),
        locale("es-ES", summary = "Organiza planes y gestiona su ejecución automática."),
        locale("hi-IN", summary = "योजनाएँ व्यवस्थित करें और उनके स्वचालित निष्पादन का प्रबंधन करें।"),
        locale("ar-SA", summary = "نظّم الخطط وأدر تنفيذها التلقائي."),
        locale("pt-BR", summary = "Organize planos e gerencie sua execução automática.")
    ),
    settings = WorkflowPlanConfig,
    settings_default = default,
)]
impl PlanPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let plan = agena_plugin_host::sdk::macro_support::parse_defaulted_settings(
            ctx.settings.clone(),
            "invalid planning config",
        )?;
        self.inner.initialize(
            ctx,
            WorkflowPluginConfig {
                tool_discovery: Default::default(),
                plan,
            },
            host,
        )?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(query, planning, read_only),
        summary = "Inspect the current plan state.",
        translations(
            locale("zh-CN", summary = "查看当前计划的状态。"),
            locale("zh-TW", summary = "查看目前計畫的狀態。"),
            locale("ja-JP", summary = "現在の計画の状態を確認します。"),
            locale("ko-KR", summary = "현재 계획의 상태를 확인합니다."),
            locale("fr-FR", summary = "Consulter l’état du plan actuel."),
            locale("de-DE", summary = "Den Status des aktuellen Plans anzeigen."),
            locale("es-ES", summary = "Consulta el estado del plan actual."),
            locale("hi-IN", summary = "मौजूदा योजना की स्थिति देखें।"),
            locale("ar-SA", summary = "اعرض حالة الخطة الحالية."),
            locale("pt-BR", summary = "Consulte o estado do plano atual.")
        )
    )]
    async fn get(&self, input: &PlanGetInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_get(input).await
    }

    #[tool(
        tags(mutate, planning),
        summary = "Create or replace the current plan without requesting approval.",
        help = "Prefer using this tool for implementation tasks unless they are simple. Use it proactively when starting a non-trivial implementation task: getting sign-off on your approach before writing code prevents wasted effort and ensures alignment. Use it when ANY of these conditions apply: new features, multiple valid approaches, changes to existing behavior or structure, architectural decisions, changes touching more than 2-3 files, unclear requirements, or when you would otherwise ask the user to clarify the approach. Only skip it for simple tasks: single-line fixes, adding a single function with clear requirements, very specific detailed instructions, or pure research/read-only work. If unsure whether to use it, err on the side of planning. This tool never blocks on the user: it saves the plan and returns. With `request_approval: true` (the default) the plan stays in the `planning` phase and you must call `plan.review` to request user approval before it becomes active. Pass `request_approval: false` only with prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation` — the plan then becomes active immediately. Never change settings to bypass approval. While the plan is in the `planning` phase, mutating tools are blocked; explore with read-only tools (including parallel `tasks.run` exploration when the scope spans multiple areas), clarify with `interaction.ask` when available, replace the plan content with `plan.set`, and update progress/notes with `plan.edit`. When the plan is complete, call `plan.review` to present it for approval; never ask whether the plan is acceptable via `interaction.ask.",
        translations(
            locale(
                "zh-CN",
                summary = "创建或替换当前计划；此工具不会请求批准。",
                help = "实施任务并不简单时先制定计划，尤其是新功能、多种可行做法、行为或架构变更、涉及多个文件或需求不清时；简单的单行修复、明确的小改动和只读研究可跳过。此工具只保存计划，不会等待用户。`request_approval` 默认是 true，计划会停留在 planning；需用 `plan.review` 请求批准后才生效。只有用户事先授权且可信设置 `agena.plan.allow_unreviewed_activation` 已启用时，才能传 false；不得改设置绕过批准。planning 阶段禁止变更工具：先只读探索，可用 `interaction.ask` 澄清，再用 `plan.set` 替换计划、`plan.edit` 更新进度。完成后用 `plan.review` 提交；不要用 `interaction.ask` 请求计划批准。"
            ),
            locale(
                "zh-TW",
                summary = "建立或取代目前計畫；此工具不會要求核准。",
                help = "實作任務並不簡單時先規劃，尤其是新功能、多種可行做法、行為或架構變更、涉及多個檔案或需求不清時；簡單單行修正、明確的小改動和唯讀研究可跳過。此工具只儲存計畫，不會等待使用者。`request_approval` 預設為 true，計畫會留在 planning；須用 `plan.review` 要求核准後才生效。只有使用者事先授權且可信設定 `agena.plan.allow_unreviewed_activation` 已啟用時，才能傳 false；不可修改設定來繞過核准。planning 階段禁止變更工具：先唯讀探索，可用 `interaction.ask` 釐清，再用 `plan.set` 取代計畫、`plan.edit` 更新進度。完成後以 `plan.review` 提交；不要用 `interaction.ask` 要求計畫核准。"
            ),
            locale(
                "ja-JP",
                summary = "承認を求めずに現在の計画を作成または置き換えます。",
                help = "単純でない実装（新機能、動作や設計の変更、複数ファイルの修正、曖昧な要件など）では先に計画してください。小さな明確な修正や読み取り専用の調査は省略できます。このツールは計画を保存して戻ります。`request_approval` は既定で true なので計画は planning のままです。有効化前に `plan.review` で承認を求めてください。false はユーザーの事前許可と信頼済み設定 `agena.plan.allow_unreviewed_activation` の両方がある場合だけ使い、設定を変えて承認を回避しないでください。planning 中は変更ツールが使えません。読み取り専用で調べ、必要なら `interaction.ask` で確認し、`plan.set` で置き換え、`plan.edit` で進捗を更新してください。完了したら `plan.review` に提出し、`interaction.ask` で計画の承認を求めないでください。"
            ),
            locale(
                "ko-KR",
                summary = "승인을 요청하지 않고 현재 계획을 만들거나 교체합니다.",
                help = "새 기능, 동작·설계 변경, 여러 파일 수정, 불명확한 요구처럼 단순하지 않은 구현 작업은 먼저 계획하세요. 작고 명확한 수정이나 읽기 전용 조사는 생략할 수 있습니다. 이 도구는 계획을 저장하고 바로 반환합니다. `request_approval`은 기본 true이므로 계획은 planning에 남습니다. 활성화 전에 `plan.review`로 승인을 요청하세요. false는 사용자의 사전 승인과 신뢰 설정 `agena.plan.allow_unreviewed_activation`이 모두 있을 때만 사용하고, 설정을 바꿔 승인을 우회하지 마세요. planning 중에는 변경 도구를 사용할 수 없습니다. 읽기 전용으로 살펴보고 필요하면 `interaction.ask`로 확인한 뒤 `plan.set`으로 교체하고 `plan.edit`로 진행 상황을 갱신하세요. 완료하면 `plan.review`에 제출하고 `interaction.ask`로 계획 승인을 요청하지 마세요."
            ),
            locale(
                "fr-FR",
                summary = "Créer ou remplacer le plan actuel sans demander d’approbation.",
                help = "Planifiez d’abord les implémentations non simples : nouvelle fonctionnalité, changement de comportement ou d’architecture, plusieurs fichiers ou exigences floues. Les petites corrections claires et recherches en lecture seule peuvent s’en passer. Cet outil enregistre le plan sans attendre l’utilisateur. `request_approval` vaut true par défaut : le plan reste en planning jusqu’à `plan.review`. La valeur false exige une autorisation préalable et le réglage fiable `agena.plan.allow_unreviewed_activation` ; ne modifiez pas les réglages pour contourner l’approbation. En planning, seuls les outils en lecture sont disponibles ; clarifiez si besoin avec `interaction.ask`, remplacez le plan avec `plan.set` et notez l’avancement avec `plan.edit`. À la fin, utilisez `plan.review` ; ne demandez pas l’approbation avec `interaction.ask`."
            ),
            locale(
                "de-DE",
                summary = "Den aktuellen Plan ohne Genehmigungsanfrage erstellen oder ersetzen.",
                help = "Planen Sie zuerst nicht einfache Implementierungen: neue Funktionen, Verhaltens- oder Architekturänderungen, mehrere Dateien oder unklare Anforderungen. Kleine eindeutige Korrekturen und Nur-Lese-Recherche können ohne Plan erfolgen. Dieses Tool speichert den Plan, ohne auf den Nutzer zu warten. `request_approval` ist standardmäßig true; der Plan bleibt in planning, bis `plan.review` die Freigabe anfragt. false erfordert vorherige Nutzerfreigabe und `agena.plan.allow_unreviewed_activation`; ändern Sie keine Einstellungen, um die Freigabe zu umgehen. In planning sind Änderungen gesperrt. Erkunden Sie nur lesend, klären Sie Bedarf mit `interaction.ask`, ersetzen Sie den Plan per `plan.set` und pflegen Sie den Fortschritt mit `plan.edit`. Reichen Sie ihn mit `plan.review` ein; fragen Sie nicht mit `interaction.ask` nach der Freigabe."
            ),
            locale(
                "es-ES",
                summary = "Crea o sustituye el plan actual sin pedir aprobación.",
                help = "Planifica primero las implementaciones que no sean sencillas: funciones nuevas, cambios de comportamiento o arquitectura, varios archivos o requisitos poco claros. Puedes omitirlo para arreglos pequeños y claros o investigación de solo lectura. Esta herramienta guarda el plan sin esperar al usuario. `request_approval` es true por defecto, así que permanece en planning hasta `plan.review`. false requiere autorización previa y el ajuste confiable `agena.plan.allow_unreviewed_activation`; no cambies ajustes para eludir la aprobación. En planning se bloquean las modificaciones: explora en modo lectura, aclara con `interaction.ask` si hace falta, sustituye con `plan.set` y actualiza el progreso con `plan.edit`. Al terminar, usa `plan.review`; no pidas aprobación con `interaction.ask`."
            ),
            locale(
                "hi-IN",
                summary = "बिना स्वीकृति माँगे मौजूदा योजना बनाएँ या बदलें।",
                help = "जटिल कार्यान्वयन—जैसे नया फ़ीचर, व्यवहार या आर्किटेक्चर बदलाव, कई फ़ाइलें या अस्पष्ट आवश्यकताएँ—से पहले योजना बनाएँ। छोटे स्पष्ट सुधार और केवल-पढ़ने वाला शोध छोड़ सकते हैं। यह टूल योजना सहेजकर लौटता है। `request_approval` डिफ़ॉल्ट true है, इसलिए योजना planning में रहती है; सक्रिय करने से पहले `plan.review` से अनुमति लें। false के लिए उपयोगकर्ता की पूर्व अनुमति और भरोसेमंद सेटिंग `agena.plan.allow_unreviewed_activation` दोनों चाहिए; अनुमति से बचने के लिए सेटिंग न बदलें। planning में बदलाव वाले टूल बंद हैं: पहले पढ़कर जाँचें, ज़रूरत पर `interaction.ask` से स्पष्ट करें, `plan.set` से योजना बदलें और `plan.edit` से प्रगति अपडेट करें। पूरा होने पर `plan.review` उपयोग करें; `interaction.ask` से स्वीकृति न माँगें।"
            ),
            locale(
                "ar-SA",
                summary = "أنشئ الخطة الحالية أو استبدلها دون طلب الموافقة.",
                help = "ضع خطة أولًا للتنفيذ غير البسيط، مثل ميزة جديدة أو تغيير في السلوك أو التصميم أو عدة ملفات أو متطلبات ملتبسة. يمكن تجاوزها للإصلاحات الصغيرة الواضحة والبحث للقراءة فقط. تحفظ الأداة الخطة دون انتظار المستخدم. `request_approval` يساوي true افتراضيًا، فتبقى الخطة في planning حتى يطلب `plan.review` الموافقة. تتطلب false إذنًا مسبقًا وإعداد `agena.plan.allow_unreviewed_activation` الموثوق؛ لا تغيّر الإعدادات لتجاوز الموافقة. تُحظر التعديلات أثناء planning: استكشف للقراءة فقط، واستوضح عند الحاجة عبر `interaction.ask`، واستبدل الخطة بـ `plan.set` وحدّث التقدم بـ `plan.edit`. قدّمها عبر `plan.review` عند الانتهاء ولا تطلب الموافقة عبر `interaction.ask`."
            ),
            locale(
                "pt-BR",
                summary = "Crie ou substitua o plano atual sem solicitar aprovação.",
                help = "Planeje primeiro implementações não triviais, como funcionalidade nova, mudança de comportamento ou arquitetura, vários arquivos ou requisitos pouco claros. Correções pequenas e claras e pesquisa somente leitura podem dispensar o plano. Esta ferramenta salva e retorna sem esperar pelo usuário. `request_approval` é true por padrão; o plano fica em planning até `plan.review`. false exige autorização prévia e a configuração confiável `agena.plan.allow_unreviewed_activation`; nunca altere configurações para contornar a aprovação. Em planning, alterações ficam bloqueadas: explore somente em leitura, esclareça com `interaction.ask` se necessário, substitua com `plan.set` e atualize o progresso com `plan.edit`. Ao concluir, use `plan.review`; não peça aprovação por `interaction.ask`."
            )
        )
    )]
    async fn set(&self, input: &PlanSetInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_set(input).await
    }

    #[tool(
        tags(mutate, planning),
        summary = "Edit the current plan's steps and checks.",
        help = "Address steps and checks by 1-based index: `step` + `status` (with an optional `note`) updates a step, `step` + `check` + `status` updates a check. This tool NEVER requests user approval and NEVER changes the plan phase — the plan stays in whatever phase it is in. Use `plan.phase` for plan-level phase transitions and `plan.review` to request approval.",
        translations(
            locale(
                "zh-CN",
                summary = "编辑当前计划中的步骤和检查项。",
                help = "步骤和检查项使用从 1 开始的索引：`step` + `status`（可选 `note`）更新步骤；`step` + `check` + `status` 更新检查项。此工具不会请求批准或更改计划阶段。阶段转换用 `plan.phase`，请求批准用 `plan.review`。"
            ),
            locale(
                "zh-TW",
                summary = "編輯目前計畫中的步驟與檢查項目。",
                help = "步驟與檢查項目使用從 1 開始的索引：`step` + `status`（可選 `note`）更新步驟；`step` + `check` + `status` 更新檢查項目。此工具不會要求核准或變更計畫階段。階段轉換用 `plan.phase`，要求核准用 `plan.review`。"
            ),
            locale(
                "ja-JP",
                summary = "現在の計画にあるステップとチェック項目を編集します。",
                help = "ステップとチェック項目は 1 始まりの番号で指定します。`step` + `status`（任意の `note` 付き）でステップを、`step` + `check` + `status` でチェック項目を更新します。このツールは承認を求めず、計画のフェーズも変更しません。フェーズ変更には `plan.phase`、承認依頼には `plan.review` を使ってください。"
            ),
            locale(
                "ko-KR",
                summary = "현재 계획의 단계와 점검 항목을 수정합니다.",
                help = "단계와 점검 항목은 1부터 시작하는 인덱스로 지정합니다. `step` + `status`(선택적 `note` 포함)는 단계를, `step` + `check` + `status`는 점검 항목을 갱신합니다. 이 도구는 승인을 요청하거나 계획 단계를 바꾸지 않습니다. 단계 전환에는 `plan.phase`, 승인 요청에는 `plan.review`를 사용하세요."
            ),
            locale(
                "fr-FR",
                summary = "Modifier les étapes et vérifications du plan actuel.",
                help = "Les étapes et contrôles utilisent un index commençant à 1 : `step` + `status` (avec `note` facultatif) modifie une étape ; `step` + `check` + `status` modifie un contrôle. Cet outil ne demande jamais d’approbation et ne change pas la phase du plan. Utilisez `plan.phase` pour changer de phase et `plan.review` pour demander une approbation."
            ),
            locale(
                "de-DE",
                summary = "Schritte und Prüfungen des aktuellen Plans bearbeiten.",
                help = "Schritte und Prüfungen verwenden einen Index ab 1: `step` + `status` (optional mit `note`) aktualisiert einen Schritt; `step` + `check` + `status` eine Prüfung. Dieses Tool fordert keine Freigabe an und ändert die Planphase nicht. Für Phasenwechsel nutzen Sie `plan.phase`, für Freigaben `plan.review`."
            ),
            locale(
                "es-ES",
                summary = "Edita los pasos y las comprobaciones del plan actual.",
                help = "Los pasos y comprobaciones usan índices desde 1: `step` + `status` (con `note` opcional) actualiza un paso; `step` + `check` + `status` actualiza una comprobación. Esta herramienta no solicita aprobación ni cambia la fase del plan. Usa `plan.phase` para cambiarla y `plan.review` para pedir aprobación."
            ),
            locale(
                "hi-IN",
                summary = "मौजूदा योजना के चरण और जाँचें संपादित करें।",
                help = "चरण और जाँच 1 से शुरू होने वाले इंडेक्स से बताएँ: `step` + `status` (वैकल्पिक `note`) चरण बदलता है; `step` + `check` + `status` जाँच बदलता है। यह टूल स्वीकृति नहीं माँगता और योजना का चरण नहीं बदलता। चरण बदलने के लिए `plan.phase`, स्वीकृति के लिए `plan.review` उपयोग करें।"
            ),
            locale(
                "ar-SA",
                summary = "عدّل خطوات الخطة الحالية وفحوصها.",
                help = "تُحدَّد الخطوات والفحوص بفهرس يبدأ من 1: يحدّث `step` مع `status` (و`note` اختياري) خطوة، بينما يحدّث `step` و`check` و`status` فحصًا. لا تطلب هذه الأداة موافقة ولا تغيّر مرحلة الخطة. استخدم `plan.phase` لتغيير المرحلة و`plan.review` لطلب الموافقة."
            ),
            locale(
                "pt-BR",
                summary = "Edite as etapas e verificações do plano atual.",
                help = "Etapas e verificações usam índices a partir de 1: `step` + `status` (com `note` opcional) atualiza uma etapa; `step` + `check` + `status` atualiza uma verificação. Esta ferramenta não solicita aprovação nem altera a fase do plano. Use `plan.phase` para mudar a fase e `plan.review` para pedir aprovação."
            )
        )
    )]
    async fn edit(&self, input: &PlanEditInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_edit(input).await
    }

    #[tool(
        tags(mutate, interactive, planning),
        summary = "Transition the current plan's phase.",
        help = "Plan-level phase transitions between `planning`, `active`, `blocked`, `completed`, and `cancelled`, with optional `autorun` and (for `completed`) `summary`. Transitions into `active`, `blocked`, or `completed` request approval by default only when the current phase is not already approved (`active`, `blocked`, or `completed`). Leave `request_approval` omitted or true for normal transitions; an already approved plan does not request another review for progress or completion. Passing `request_approval: false` requires prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation`; never change settings to bypass approval. To complete a plan with steps, mark the required steps/checks `completed` via `plan.edit` first, then call this tool separately with `phase: completed`.",
        translations(
            locale(
                "zh-CN",
                summary = "转换当前计划的阶段。",
                help = "阶段包括 `planning`、`active`、`blocked`、`completed` 和 `cancelled`，可选设置 `autorun`，完成时还可附 `summary`。切换到 active、blocked 或 completed 只有在当前阶段未获批准时才默认请求批准；常规操作省略 `request_approval` 或设为 true。设为 false 必须同时有用户事先授权和可信设置 `agena.plan.allow_unreviewed_activation`，不得修改设置绕过批准。若计划包含步骤，先用 `plan.edit` 将要求的步骤和检查项标记为 completed，再单独用 `phase: completed` 调用本工具。"
            ),
            locale(
                "zh-TW",
                summary = "切換目前計畫的階段。",
                help = "階段包括 `planning`、`active`、`blocked`、`completed` 和 `cancelled`，可選設定 `autorun`，完成時也可附 `summary`。切換至 active、blocked 或 completed 只有在目前階段尚未核准時才會預設要求核准；一般操作省略 `request_approval` 或設為 true。設為 false 必須同時有使用者事先授權及可信設定 `agena.plan.allow_unreviewed_activation`，不可修改設定繞過核准。若計畫含步驟，先用 `plan.edit` 將必要步驟與檢查標記為 completed，再另外以 `phase: completed` 呼叫本工具。"
            ),
            locale(
                "ja-JP",
                summary = "現在の計画のフェーズを変更します。",
                help = "フェーズは `planning`、`active`、`blocked`、`completed`、`cancelled` です。`autorun` は任意で、completed には `summary` も指定できます。active・blocked・completed への変更は、未承認の場合のみ既定で承認を求めます。通常は `request_approval` を省略するか true にしてください。false にはユーザーの事前許可と信頼済み設定 `agena.plan.allow_unreviewed_activation` の両方が必要です。承認回避のため設定を変えないでください。ステップがある計画を完了する場合は、先に `plan.edit` で必要な項目を completed にし、その後 `phase: completed` で別途呼び出してください。"
            ),
            locale(
                "ko-KR",
                summary = "현재 계획의 단계를 전환합니다.",
                help = "단계는 `planning`, `active`, `blocked`, `completed`, `cancelled`입니다. `autorun`은 선택 항목이며 completed에는 `summary`도 추가할 수 있습니다. active, blocked, completed로 이동할 때 아직 승인되지 않은 경우에만 기본적으로 승인을 요청합니다. 일반 전환에서는 `request_approval`을 생략하거나 true로 두세요. false는 사용자 사전 승인과 신뢰 설정 `agena.plan.allow_unreviewed_activation`이 모두 있어야 합니다. 승인을 우회하려고 설정을 바꾸지 마세요. 단계가 있는 계획은 먼저 `plan.edit`로 필요한 항목을 completed로 표시한 뒤 `phase: completed`로 별도 호출하세요."
            ),
            locale(
                "fr-FR",
                summary = "Changer la phase du plan actuel.",
                help = "Phases possibles : `planning`, `active`, `blocked`, `completed`, `cancelled`. `autorun` est facultatif et `summary` peut accompagner completed. Les changements vers active, blocked ou completed demandent une approbation uniquement si la phase actuelle n’est pas déjà approuvée. Omettez `request_approval` ou laissez-le à true pour un changement normal. false exige l’autorisation préalable de l’utilisateur et le réglage fiable `agena.plan.allow_unreviewed_activation` ; ne modifiez pas les réglages pour contourner l’approbation. Pour terminer un plan avec étapes, marquez d’abord les éléments requis completed avec `plan.edit`, puis appelez séparément avec `phase: completed`."
            ),
            locale(
                "de-DE",
                summary = "Die Phase des aktuellen Plans wechseln.",
                help = "Mögliche Phasen: `planning`, `active`, `blocked`, `completed`, `cancelled`. `autorun` ist optional; bei completed kann `summary` ergänzt werden. Übergänge zu active, blocked oder completed fordern nur dann eine Freigabe an, wenn der Plan noch nicht freigegeben ist. Lassen Sie `request_approval` weg oder setzen Sie es auf true. false erfordert vorherige Nutzerfreigabe und die vertrauenswürdige Einstellung `agena.plan.allow_unreviewed_activation`; ändern Sie keine Einstellungen, um Freigaben zu umgehen. Markieren Sie vor Abschluss eines Plans mit Schritten die nötigen Punkte per `plan.edit` als completed und rufen Sie danach separat mit `phase: completed` auf."
            ),
            locale(
                "es-ES",
                summary = "Cambia la fase del plan actual.",
                help = "Fases: `planning`, `active`, `blocked`, `completed` y `cancelled`. `autorun` es opcional y puedes incluir `summary` al completar. El paso a active, blocked o completed solicita aprobación solo si la fase actual aún no está aprobada. En transiciones normales, omite `request_approval` o usa true. false requiere autorización previa del usuario y el ajuste confiable `agena.plan.allow_unreviewed_activation`; no cambies ajustes para eludir la aprobación. Para completar un plan con pasos, márcalos primero como completed con `plan.edit` y después llama por separado con `phase: completed`."
            ),
            locale(
                "hi-IN",
                summary = "मौजूदा योजना का चरण बदलें।",
                help = "चरण हैं `planning`, `active`, `blocked`, `completed` और `cancelled`; `autorun` वैकल्पिक है और completed के साथ `summary` दे सकते हैं। active, blocked या completed पर जाने पर तभी स्वीकृति माँगी जाती है जब मौजूदा योजना पहले स्वीकृत न हो। सामान्य बदलाव में `request_approval` छोड़ें या true रखें। false के लिए उपयोगकर्ता की पूर्व अनुमति और भरोसेमंद सेटिंग `agena.plan.allow_unreviewed_activation` दोनों चाहिए; अनुमति से बचने के लिए सेटिंग न बदलें। चरणों वाली योजना पूरी करने से पहले `plan.edit` से ज़रूरी चरण completed करें, फिर `phase: completed` के साथ अलग कॉल करें।"
            ),
            locale(
                "ar-SA",
                summary = "غيّر مرحلة الخطة الحالية.",
                help = "المراحل هي `planning` و`active` و`blocked` و`completed` و`cancelled`. يمكن تحديد `autorun` اختياريًا، و`summary` عند الإكمال. لا يُطلب اعتماد الانتقال إلى active أو blocked أو completed افتراضيًا إلا إذا كانت الخطة غير معتمدة بعد. اترك `request_approval` محذوفًا أو true عادةً. تتطلب false إذنًا مسبقًا من المستخدم والإعداد الموثوق `agena.plan.allow_unreviewed_activation`؛ لا تغيّر الإعدادات لتجاوز الموافقة. لإكمال خطة ذات خطوات، علّم المطلوب completed عبر `plan.edit` أولًا ثم استدعِ الأداة منفصلة مع `phase: completed`."
            ),
            locale(
                "pt-BR",
                summary = "Altere a fase do plano atual.",
                help = "Fases: `planning`, `active`, `blocked`, `completed` e `cancelled`. `autorun` é opcional e `summary` pode ser informado ao concluir. A mudança para active, blocked ou completed só pede aprovação por padrão quando o plano ainda não foi aprovado. Em mudanças normais, omita `request_approval` ou use true. false exige autorização prévia do usuário e a configuração confiável `agena.plan.allow_unreviewed_activation`; nunca altere configurações para contornar a aprovação. Para concluir um plano com etapas, marque primeiro as etapas necessárias como completed com `plan.edit` e depois chame separadamente com `phase: completed`."
            )
        )
    )]
    async fn phase(&self, input: &PlanPhaseInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_phase(input).await
    }

    #[tool(
        tags(mutate, interactive, planning),
        summary = "Request user approval of the current plan before it becomes active.",
        help = "Request user approval of the current saved plan; this may pause for the user. The plan.phase tool also requests review for transitions that need approval. It reviews the current saved plan and, when the user approves, moves it from `planning` to `active`. Call it after creating or refining the plan with `plan.set` / `plan.edit`. If the user leaves feedback or rejects, the plan stays in `planning` so you can revise it and propose again.",
        translations(
            locale(
                "zh-CN",
                summary = "在计划生效前请求用户批准。",
                help = "请求用户审阅并批准已保存的当前计划；等待回复时流程可能暂停。需要批准的阶段转换也可由 `plan.phase` 发起审核。用户批准后，计划从 `planning` 变为 `active`。请在用 `plan.set` 或 `plan.edit` 创建或完善计划后调用。若用户给出意见或拒绝，计划仍在 planning，可修改后再次提交。"
            ),
            locale(
                "zh-TW",
                summary = "在計畫生效前請使用者核准。",
                help = "請使用者審閱並核准已儲存的目前計畫；等待回覆時流程可能暫停。需要核准的階段轉換也可由 `plan.phase` 發起審查。使用者核准後，計畫會從 `planning` 變為 `active`。請在使用 `plan.set` 或 `plan.edit` 建立或完善計畫後呼叫。若使用者提供意見或拒絕，計畫仍留在 planning，可修改後再次提交。"
            ),
            locale(
                "ja-JP",
                summary = "計画を有効にする前にユーザーの承認を求めます。",
                help = "保存済みの計画についてユーザーに承認を求めます。応答待ちで作業が一時停止することがあります。承認が必要なフェーズ変更は `plan.phase` からもレビューを依頼できます。承認されると計画は `planning` から `active` に移ります。`plan.set` または `plan.edit` で作成・修正した後に呼び出してください。フィードバックや却下があれば planning のままなので、修正して再提案できます。"
            ),
            locale(
                "ko-KR",
                summary = "계획을 활성화하기 전에 사용자 승인을 요청합니다.",
                help = "저장된 현재 계획을 검토하고 승인해 달라고 요청합니다. 답변을 기다리는 동안 작업이 일시 중지될 수 있습니다. 승인이 필요한 단계 전환은 `plan.phase`에서도 검토를 요청할 수 있습니다. 승인되면 계획은 `planning`에서 `active`로 이동합니다. `plan.set` 또는 `plan.edit`로 계획을 만들거나 다듬은 뒤 호출하세요. 의견이나 거절이 있으면 planning에 남으므로 수정해 다시 제안할 수 있습니다."
            ),
            locale(
                "fr-FR",
                summary = "Demander l’approbation du plan avant son activation.",
                help = "Demande à l’utilisateur d’examiner et d’approuver le plan enregistré ; l’attente peut suspendre le travail. `plan.phase` sollicite aussi une revue pour les transitions qui l’exigent. Après approbation, le plan passe de `planning` à `active`. Appelez cet outil après l’avoir créé ou affiné avec `plan.set` ou `plan.edit`. En cas de retour ou de refus, il reste en planning pour être révisé et soumis à nouveau."
            ),
            locale(
                "de-DE",
                summary = "Vor der Aktivierung die Freigabe des aktuellen Plans anfragen.",
                help = "Bittet den Nutzer um Prüfung und Freigabe des gespeicherten Plans; währenddessen kann der Ablauf warten. `plan.phase` kann bei freigabepflichtigen Übergängen ebenfalls eine Prüfung anfragen. Nach Zustimmung wechselt der Plan von `planning` zu `active`. Rufen Sie das Tool nach Erstellung oder Überarbeitung mit `plan.set` oder `plan.edit` auf. Bei Feedback oder Ablehnung bleibt der Plan in planning und kann angepasst erneut vorgelegt werden."
            ),
            locale(
                "es-ES",
                summary = "Solicita aprobación del plan actual antes de activarlo.",
                help = "Pide al usuario que revise y apruebe el plan guardado; la espera puede pausar el trabajo. `plan.phase` también solicita revisión para las transiciones que la requieren. Si se aprueba, el plan pasa de `planning` a `active`. Llama a esta herramienta después de crearlo o mejorarlo con `plan.set` o `plan.edit`. Si el usuario comenta o rechaza, permanece en planning para que puedas revisarlo y volver a proponerlo."
            ),
            locale(
                "hi-IN",
                summary = "योजना सक्रिय करने से पहले उपयोगकर्ता की स्वीकृति माँगें।",
                help = "सहेजी गई मौजूदा योजना की समीक्षा और स्वीकृति माँगें; उत्तर की प्रतीक्षा में काम रुक सकता है। जिन चरण परिवर्तनों में स्वीकृति चाहिए, वहाँ `plan.phase` भी समीक्षा माँग सकता है। स्वीकृति मिलने पर योजना `planning` से `active` में जाती है। `plan.set` या `plan.edit` से बनाने या सुधारने के बाद बुलाएँ। उपयोगकर्ता सुझाव दे या मना करे तो योजना planning में रहेगी, ताकि आप उसे सुधारकर फिर प्रस्तुत कर सकें।"
            ),
            locale(
                "ar-SA",
                summary = "اطلب موافقة المستخدم على الخطة قبل تفعيلها.",
                help = "اطلب من المستخدم مراجعة الخطة المحفوظة الحالية والموافقة عليها؛ وقد يتوقف العمل أثناء الانتظار. يمكن لـ `plan.phase` طلب المراجعة أيضًا عند الانتقالات التي تحتاج إلى اعتماد. بعد الموافقة تنتقل الخطة من `planning` إلى `active`. استدع هذه الأداة بعد إنشاء الخطة أو تحسينها عبر `plan.set` أو `plan.edit`. إذا قدّم المستخدم ملاحظات أو رفضها، تبقى في planning لتعديلها واقتراحها مجددًا."
            ),
            locale(
                "pt-BR",
                summary = "Peça aprovação do plano atual antes de ativá-lo.",
                help = "Solicite ao usuário que revise e aprove o plano salvo; a espera pode pausar o trabalho. `plan.phase` também solicita revisão nas transições que precisam de aprovação. Depois de aprovado, o plano passa de `planning` para `active`. Chame esta ferramenta depois de criar ou aprimorar o plano com `plan.set` ou `plan.edit`. Se o usuário enviar comentários ou rejeitar, o plano fica em planning para ser ajustado e apresentado novamente."
            )
        )
    )]
    async fn review(&self, input: &PlanReviewInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_review(input).await
    }

    #[tool(
        tags(mutate, planning),
        summary = "Remove the current plan.",
        translations(
            locale("zh-CN", summary = "删除当前计划。"),
            locale("zh-TW", summary = "移除目前計畫。"),
            locale("ja-JP", summary = "現在の計画を削除します。"),
            locale("ko-KR", summary = "현재 계획을 삭제합니다."),
            locale("fr-FR", summary = "Supprimer le plan actuel."),
            locale("de-DE", summary = "Den aktuellen Plan entfernen."),
            locale("es-ES", summary = "Elimina el plan actual."),
            locale("hi-IN", summary = "मौजूदा योजना हटाएँ।"),
            locale("ar-SA", summary = "أزل الخطة الحالية."),
            locale("pt-BR", summary = "Remova o plano atual.")
        )
    )]
    async fn clear(&self) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_clear().await
    }

    #[hook(tool.before)]
    async fn tool_execute_before(
        &self,
        input: ToolBeforeInput,
    ) -> SdkResult<Option<ToolBeforePatch>> {
        self.inner.tool_execute_before_hook(input).await
    }

    #[hook(shell.before)]
    async fn command_execute_before(
        &self,
        input: CommandBeforeInput,
    ) -> SdkResult<Option<CommandBeforeResponse>> {
        self.inner.command_execute_before_hook(input).await
    }

    #[hook(agent.stop)]
    async fn agent_stop(
        &self,
        input: agena_plugin_host::AgentStopInput,
    ) -> SdkResult<Option<agena_plugin_host::AgentStopPatch>> {
        self.inner.agent_stop_hook(input).await
    }

    #[hook(agent.cancel)]
    async fn agent_cancel(&self, input: agena_plugin_host::AgentCancelInput) -> SdkResult<()> {
        self.inner.agent_cancel_hook(input).await
    }
}
