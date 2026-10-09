//! `agena.cron` plugin: schedules cron jobs.
//!
//! The model-visible schedule tools execute through the same plugin tool
//! surface as every other tool.

use std::sync::{Arc, RwLock};

use crate::part::{
    CronCreateToolInput, CronDeleteToolInput, CronHistoryToolInput, CronJobControlToolInput,
    CronListToolInput, CronUpdateToolInput,
};
use crate::plugins::provided::router;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeOutput};

pub(crate) const CRON_PLUGIN_ID: &str = "agena.cron";

pub(crate) struct CronPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "cron",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Cron-style and one-shot wakeup scheduling tools.",
    translations(
        locale("zh-CN", summary = "创建周期性或一次性的定时唤醒任务。"),
        locale("zh-TW", summary = "建立週期性或單次執行的排程喚醒工作。"),
        locale("ja-JP", summary = "定期または一度だけのセッション起動をスケジュールします。"),
        locale("ko-KR", summary = "반복 또는 일회성 세션 깨우기를 예약합니다."),
        locale("fr-FR", summary = "Planifier des réveils récurrents ou ponctuels de la session."),
        locale("de-DE", summary = "Wiederkehrende oder einmalige Sitzungsweckrufe planen."),
        locale("es-ES", summary = "Programa avisos periódicos o puntuales para reactivar la sesión."),
        locale("hi-IN", summary = "बार-बार या एक बार होने वाले सत्र वेकअप शेड्यूल करें।"),
        locale("ar-SA", summary = "جدول تنبيهات دورية أو لمرة واحدة لإيقاظ الجلسة."),
        locale("pt-BR", summary = "Agende despertares recorrentes ou pontuais da sessão.")
    ),
)]
impl CronPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
        }
    }

    #[hook(init)]
    async fn init(
        &self,
        _ctx: agena_plugin_host::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<agena_plugin_host::sdk::InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::internal("cron plugin host lock poisoned"))? = Some(host);
        Ok(agena_plugin_host::sdk::InitOutcome::ack(
            agena_plugin_host::sdk::Plugin::manifest(self),
        ))
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::internal("cron plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::internal("cron plugin invoked before init"))
    }

    #[tool(
        tags(query, scheduler, discovery, read_only),
        summary = "List registered cron jobs and wakeups.",
        help = "List every scheduled job registered in this session. Jobs are session-only — they exist for this session's lifetime and are gone when it ends — and recurring jobs auto-expire after seven days. Use this to review schedules you created; never poll it waiting for a job to fire.",
        translations(
            locale(
                "zh-CN",
                summary = "列出已登记的定时任务和唤醒计划。",
                help = "列出此会话中登记的所有计划任务。任务仅在当前会话有效，会话结束后即删除；周期任务会在七天后自动过期。用此工具查看已创建的计划，不要反复查询来等待任务触发。"
            ),
            locale(
                "zh-TW",
                summary = "列出已登記的排程工作與喚醒計畫。",
                help = "列出此工作階段中登記的所有排程工作。工作只在目前工作階段有效，結束後便會移除；週期工作會在七天後自動到期。用此工具查看已建立的排程，請勿反覆查詢等待工作觸發。"
            ),
            locale(
                "ja-JP",
                summary = "登録済みの cron ジョブとセッション起動予定を一覧表示します。",
                help = "このセッションに登録されたすべての予定を一覧表示します。予定はセッション終了時に削除され、繰り返し予定は 7 日後に自動で期限切れになります。作成済みの予定の確認に使い、実行を待つために繰り返し呼び出さないでください。"
            ),
            locale(
                "ko-KR",
                summary = "등록된 cron 작업과 세션 깨우기 예약을 나열합니다.",
                help = "이 세션에 등록된 모든 예약 작업을 나열합니다. 작업은 현재 세션에서만 유지되며 세션이 끝나면 사라집니다. 반복 작업은 7일 후 자동 만료됩니다. 예약을 확인할 때 사용하고, 실행을 기다리며 반복 호출하지 마세요."
            ),
            locale(
                "fr-FR",
                summary = "Lister les tâches cron et les réveils planifiés.",
                help = "Liste toutes les tâches planifiées de cette session. Elles ne vivent que pendant la session et les tâches récurrentes expirent au bout de sept jours. Utilisez cet outil pour vérifier vos horaires ; ne l’interrogez jamais en boucle en attendant un déclenchement."
            ),
            locale(
                "de-DE",
                summary = "Registrierte Cron-Jobs und geplante Sitzungsweckrufe auflisten.",
                help = "Listet alle für diese Sitzung registrierten Zeitpläne auf. Sie gelten nur für die Dauer der Sitzung; wiederkehrende Jobs laufen nach sieben Tagen automatisch ab. Verwenden Sie die Liste zur Kontrolle Ihrer Zeitpläne und fragen Sie sie nicht wiederholt ab, um auf einen Lauf zu warten."
            ),
            locale(
                "es-ES",
                summary = "Enumera las tareas cron y los avisos de reactivación registrados.",
                help = "Enumera todas las tareas programadas de esta sesión. Solo existen mientras dure la sesión y las tareas recurrentes caducan automáticamente a los siete días. Usa esta herramienta para revisar tus horarios; no la consultes repetidamente para esperar a que se ejecute una tarea."
            ),
            locale(
                "hi-IN",
                summary = "रजिस्टर किए गए cron काम और सत्र जगाने के शेड्यूल दिखाएँ।",
                help = "इस सत्र में रजिस्टर किए गए सभी शेड्यूल दिखाएँ। ये केवल सत्र रहने तक मौजूद रहते हैं; बार-बार चलने वाले काम सात दिन बाद अपने-आप समाप्त हो जाते हैं। बनाए गए शेड्यूल देखने के लिए इसका उपयोग करें; काम चलने की प्रतीक्षा में इसे बार-बार न पूछें।"
            ),
            locale(
                "ar-SA",
                summary = "اعرض مهام cron المجدولة وتنبيهات إيقاظ الجلسة المسجلة.",
                help = "اعرض كل المهام المجدولة في هذه الجلسة. تظل المهام مرتبطة بعمر الجلسة وتُحذف عند انتهائها، كما تنتهي المهام المتكررة تلقائيًا بعد سبعة أيام. استخدم الأداة لمراجعة الجداول التي أنشأتها، ولا تستطلعها مرارًا انتظارًا للتنفيذ."
            ),
            locale(
                "pt-BR",
                summary = "Liste as tarefas cron e os despertares de sessão agendados.",
                help = "Lista todas as tarefas agendadas nesta sessão. Elas existem apenas enquanto a sessão estiver ativa, e as recorrentes expiram automaticamente após sete dias. Use a lista para conferir os agendamentos criados; não consulte repetidamente enquanto espera uma execução."
            )
        )
    )]
    async fn invoke_list(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
        args: CronListToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_list",
            serde_json::to_value(args).map_err(|err| PluginError::invalid_params_error(&err))?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate, scheduler),
        summary = "Create one cron schedule.",
        help = "Schedule a recurring wake with a 6-field cron expression (second minute hour day-of-month month day-of-week). Always pass the IANA timezone from environment_context; wall-clock fields are evaluated in that timezone and returned times are explicit RFC 3339 instants. When the job fires while the session is idle, its prompt is appended chronologically as a typed system_notification and wakes the model; never use it to poll. Jobs are session-only and auto-expire after seven days. When the exact time does not matter, avoid :00 and :30 to reduce clumping.",
        translations(
            locale(
                "zh-CN",
                summary = "创建一条 cron 定时计划。",
                help = "使用六段 cron 表达式创建周期唤醒：秒、分、时、日、月、星期。必须传入 environment_context 中的 IANA 时区；计划按该时区解释本地时间，返回的时间是明确的 RFC 3339 时刻。会话空闲时触发，提示会按时间顺序作为带类型的 system_notification 加入会话并唤醒模型；不要用它轮询。计划仅限当前会话，七天后自动过期。若无需精确到某一时刻，尽量避开 :00 和 :30，减少任务集中触发。"
            ),
            locale(
                "zh-TW",
                summary = "建立一筆 cron 排程。",
                help = "使用六欄 cron 表達式建立週期性喚醒：秒、分、小時、日期、月份、星期。務必傳入 environment_context 中的 IANA 時區；排程會依該時區解讀本地時間，回傳時間則是明確的 RFC 3339 時刻。工作階段閒置時若排程觸發，提示會依時間順序以具型別的 system_notification 加入工作階段並喚醒模型；不要用它輪詢。排程只屬於目前工作階段，七天後自動到期。不必精確到某個時刻時，避開 :00 和 :30，避免工作集中觸發。"
            ),
            locale(
                "ja-JP",
                summary = "cron スケジュールを 1 件作成します。",
                help = "秒・分・時・日・月・曜日の 6 フィールド cron 式で、定期的な起動を設定します。environment_context にある IANA タイムゾーンを必ず指定してください。時刻はそのタイムゾーンで解釈され、返却値は RFC 3339 の明確な時刻です。セッションがアイドル中に実行時刻になると、プロンプトが時系列に沿って型付き system_notification として追加され、モデルを起動します。ポーリング用途には使わないでください。予定はこのセッション限りで、7 日後に自動失効します。時刻を厳密に指定する必要がなければ、実行の集中を避けるため :00 と :30 を避けてください。"
            ),
            locale(
                "ko-KR",
                summary = "cron 예약 하나를 만듭니다.",
                help = "초, 분, 시, 일, 월, 요일의 6개 필드 cron 식으로 반복 깨우기를 예약합니다. environment_context에 있는 IANA 시간대를 반드시 전달하세요. 현지 시각 필드는 해당 시간대로 해석되며 반환 시각은 RFC 3339 절대 시각입니다. 세션이 유휴 상태일 때 작업이 실행되면 프롬프트가 시간순 typed system_notification으로 추가되어 모델을 깨웁니다. 폴링 용도로 사용하지 마세요. 예약은 이 세션에서만 유효하며 7일 후 자동 만료됩니다. 정확한 시각이 중요하지 않다면 작업이 몰리지 않도록 :00과 :30을 피하세요."
            ),
            locale(
                "fr-FR",
                summary = "Créer une tâche cron.",
                help = "Planifiez un réveil récurrent avec une expression cron à 6 champs (seconde, minute, heure, jour du mois, mois, jour de la semaine). Indiquez toujours le fuseau IANA de environment_context : les heures locales sont évaluées dans ce fuseau et les instants renvoyés sont au format RFC 3339. Si la session est inactive au déclenchement, le prompt est ajouté dans l’ordre chronologique comme system_notification typée et réveille le modèle ; ne l’utilisez jamais pour sonder. Les tâches sont limitées à la session et expirent après sept jours. Si l’heure exacte importe peu, évitez :00 et :30 pour répartir les déclenchements."
            ),
            locale(
                "de-DE",
                summary = "Einen Cron-Zeitplan erstellen.",
                help = "Planen Sie einen wiederkehrenden Weckruf mit einem Cron-Ausdruck aus sechs Feldern (Sekunde, Minute, Stunde, Monatstag, Monat, Wochentag). Geben Sie immer die IANA-Zeitzone aus environment_context an. Lokale Uhrzeiten werden in dieser Zeitzone ausgewertet; zurückgegebene Zeitpunkte sind eindeutige RFC-3339-Zeitstempel. Löst der Job bei inaktiver Sitzung aus, wird sein Prompt chronologisch als typisierte system_notification angefügt und weckt das Modell. Verwenden Sie den Zeitplan nicht zum Polling. Jobs gelten nur für diese Sitzung und laufen nach sieben Tagen ab. Wenn die genaue Uhrzeit unwichtig ist, vermeiden Sie :00 und :30, damit sich Ausführungen weniger häufen."
            ),
            locale(
                "es-ES",
                summary = "Crea una programación cron.",
                help = "Programa una reactivación periódica con una expresión cron de seis campos (segundo, minuto, hora, día del mes, mes y día de la semana). Indica siempre la zona horaria IANA de environment_context: las horas locales se interpretan en esa zona y los instantes devueltos usan RFC 3339. Si se ejecuta cuando la sesión está inactiva, el aviso se añade en orden cronológico como system_notification tipificada y despierta al modelo; no lo uses para consultar el estado repetidamente. La tarea solo pertenece a esta sesión y caduca a los siete días. Si la hora exacta no importa, evita :00 y :30 para repartir mejor las ejecuciones."
            ),
            locale(
                "hi-IN",
                summary = "एक cron शेड्यूल बनाएँ।",
                help = "छह फ़ील्ड वाली cron अभिव्यक्ति (सेकंड, मिनट, घंटा, महीने का दिन, महीना, सप्ताह का दिन) से बार-बार होने वाला वेकअप तय करें। environment_context से IANA समय क्षेत्र हमेशा दें; स्थानीय समय उसी क्षेत्र में समझे जाते हैं और लौटाए गए समय स्पष्ट RFC 3339 क्षण होते हैं। सत्र निष्क्रिय होने पर काम चलने पर उसका प्रॉम्प्ट समयक्रम में typed system_notification के रूप में जुड़ता है और मॉडल को जगाता है; इसे पोलिंग के लिए न चलाएँ। शेड्यूल केवल इसी सत्र के लिए है और सात दिन बाद समाप्त हो जाता है। सटीक समय ज़रूरी न हो तो काम एक साथ न चलें, इसके लिए :00 और :30 से बचें।"
            ),
            locale(
                "ar-SA",
                summary = "أنشئ جدول cron واحدًا.",
                help = "جدول تنبيهًا متكررًا بتعبير cron من ستة حقول: الثانية والدقيقة والساعة ويوم الشهر والشهر ويوم الأسبوع. مرّر دائمًا منطقة IANA الزمنية من environment_context؛ تُفسَّر أوقات الساعة المحلية وفقها وتُعاد اللحظات بصيغة RFC 3339 الواضحة. إذا نُفذت المهمة والجلسة خاملة، يُضاف طلبها زمنيًا كإشعار system_notification ذي نوع محدد ويوقظ النموذج؛ لا تستخدمها للاستطلاع المتكرر. تقتصر المهام على الجلسة وتنتهي تلقائيًا بعد سبعة أيام. إن لم تكن الدقة مهمة، فتجنب :00 و:30 لتوزيع أوقات التنفيذ."
            ),
            locale(
                "pt-BR",
                summary = "Crie um agendamento cron.",
                help = "Agende um despertar recorrente com uma expressão cron de seis campos (segundo, minuto, hora, dia do mês, mês e dia da semana). Sempre informe o fuso IANA de environment_context. Os horários locais são avaliados nesse fuso, e os instantes retornados são explícitos no formato RFC 3339. Se a tarefa disparar enquanto a sessão estiver ociosa, o prompt será anexado em ordem cronológica como system_notification tipificada e despertará o modelo; não use isso para consultar em loop. A tarefa vale só para esta sessão e expira após sete dias. Se o horário exato não for importante, evite :00 e :30 para distribuir melhor as execuções."
            )
        )
    )]
    async fn invoke_create(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
        args: CronCreateToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_create",
            serde_json::to_value(args).map_err(|err| PluginError::invalid_params_error(&err))?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate, scheduler),
        summary = "Delete one cron schedule.",
        help = "Permanently remove a scheduled job from this session. Deleting stops future firings immediately.",
        translations(
            locale(
                "zh-CN",
                summary = "删除一条 cron 定时计划。",
                help = "从当前会话永久删除一项计划任务，删除后会立即停止后续触发。"
            ),
            locale(
                "zh-TW",
                summary = "刪除一筆 cron 排程。",
                help = "從目前工作階段永久移除排程工作，刪除後會立即停止後續觸發。"
            ),
            locale(
                "ja-JP",
                summary = "cron スケジュールを削除します。",
                help = "このセッションから予定を完全に削除します。削除すると今後の実行は直ちに停止します。"
            ),
            locale(
                "ko-KR",
                summary = "cron 예약 하나를 삭제합니다.",
                help = "현재 세션에서 예약 작업을 영구 삭제합니다. 삭제하면 이후 실행은 즉시 중단됩니다."
            ),
            locale(
                "fr-FR",
                summary = "Supprimer une tâche cron.",
                help = "Supprime définitivement une tâche planifiée de cette session. Ses prochains déclenchements cessent immédiatement."
            ),
            locale(
                "de-DE",
                summary = "Einen Cron-Zeitplan löschen.",
                help = "Entfernt einen geplanten Job dauerhaft aus dieser Sitzung. Künftige Ausführungen werden sofort gestoppt."
            ),
            locale(
                "es-ES",
                summary = "Elimina una programación cron.",
                help = "Elimina definitivamente una tarea programada de esta sesión. Las próximas ejecuciones se detienen de inmediato."
            ),
            locale(
                "hi-IN",
                summary = "एक cron शेड्यूल हटाएँ।",
                help = "इस सत्र से शेड्यूल किए गए काम को स्थायी रूप से हटाएँ। हटाते ही आगे के सभी निष्पादन रुक जाएँगे।"
            ),
            locale(
                "ar-SA",
                summary = "احذف جدول cron.",
                help = "أزل المهمة المجدولة نهائيًا من هذه الجلسة؛ يتوقف أي تنفيذ مستقبلي فورًا."
            ),
            locale(
                "pt-BR",
                summary = "Exclua um agendamento cron.",
                help = "Remova permanentemente uma tarefa agendada desta sessão. As próximas execuções param imediatamente."
            )
        )
    )]
    async fn invoke_delete(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
        args: CronDeleteToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_delete",
            serde_json::to_value(args).map_err(|err| PluginError::invalid_params_error(&err))?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate, scheduler),
        summary = "Update the prompt or schedule parameters of one retained job.",
        help = "Change the prompt or cron parameters of an existing job. The updated schedule takes effect for subsequent firings.",
        translations(
            locale(
                "zh-CN",
                summary = "更新现有计划任务的提示或定时参数。",
                help = "修改现有任务的提示或 cron 参数。更新后的计划从下一次触发开始生效。"
            ),
            locale(
                "zh-TW",
                summary = "更新既有排程工作的提示或排程參數。",
                help = "修改既有工作的提示或 cron 參數。更新後的排程會從下次觸發起生效。"
            ),
            locale(
                "ja-JP",
                summary = "既存ジョブのプロンプトまたはスケジュールを更新します。",
                help = "既存ジョブのプロンプトまたは cron 設定を変更します。更新内容は次回以降の実行に適用されます。"
            ),
            locale(
                "ko-KR",
                summary = "기존 작업의 프롬프트나 예약 설정을 변경합니다.",
                help = "기존 작업의 프롬프트 또는 cron 설정을 바꿉니다. 변경 내용은 다음 실행부터 적용됩니다."
            ),
            locale(
                "fr-FR",
                summary = "Modifier le prompt ou le calendrier d’une tâche existante.",
                help = "Modifie le prompt ou les paramètres cron d’une tâche existante. Les changements s’appliquent aux prochains déclenchements."
            ),
            locale(
                "de-DE",
                summary = "Prompt oder Zeitplan eines bestehenden Jobs ändern.",
                help = "Ändert den Prompt oder die Cron-Parameter eines vorhandenen Jobs. Die Änderung gilt ab den nächsten Ausführungen."
            ),
            locale(
                "es-ES",
                summary = "Actualiza el prompt o el horario de una tarea existente.",
                help = "Cambia el prompt o los parámetros cron de una tarea existente. El cambio se aplica a las próximas ejecuciones."
            ),
            locale(
                "hi-IN",
                summary = "किसी मौजूदा काम का प्रॉम्प्ट या शेड्यूल बदलें।",
                help = "मौजूदा काम का प्रॉम्प्ट या cron पैरामीटर बदलें। बदलाव अगली बार काम चलने से लागू होगा।"
            ),
            locale(
                "ar-SA",
                summary = "حدّث طلب مهمة محفوظة أو إعدادات جدولها.",
                help = "غيّر الطلب أو إعدادات cron لمهمة موجودة. تسري التغييرات على مرات التنفيذ التالية."
            ),
            locale(
                "pt-BR",
                summary = "Atualize o prompt ou o agendamento de uma tarefa existente.",
                help = "Altere o prompt ou os parâmetros cron de uma tarefa existente. A mudança vale a partir das próximas execuções."
            )
        )
    )]
    async fn invoke_update(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
        args: CronUpdateToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_update",
            serde_json::to_value(args).map_err(|err| PluginError::invalid_params_error(&err))?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate, scheduler),
        summary = "Pause one scheduled job without deleting it.",
        help = "Temporarily suspend a job's future firings while keeping its definition. Use resume to start it again.",
        translations(
            locale(
                "zh-CN",
                summary = "暂停一条计划任务，但保留其设置。",
                help = "暂时停止任务后续触发，但保留任务定义。使用 resume 可重新启用。"
            ),
            locale(
                "zh-TW",
                summary = "暫停一筆排程工作，但保留其設定。",
                help = "暫時停止工作的後續觸發，但保留工作定義。使用 resume 即可重新啟用。"
            ),
            locale(
                "ja-JP",
                summary = "スケジュールを削除せず一時停止します。",
                help = "ジョブの定義を残したまま、今後の実行を一時停止します。再開するには resume を使います。"
            ),
            locale(
                "ko-KR",
                summary = "예약 작업을 삭제하지 않고 일시 중지합니다.",
                help = "작업 정의는 유지하면서 이후 실행을 일시 중지합니다. 다시 시작하려면 resume을 사용하세요."
            ),
            locale(
                "fr-FR",
                summary = "Mettre une tâche planifiée en pause sans la supprimer.",
                help = "Suspend temporairement les prochains déclenchements tout en conservant la définition. Utilisez resume pour la réactiver."
            ),
            locale(
                "de-DE",
                summary = "Einen geplanten Job pausieren, ohne ihn zu löschen.",
                help = "Unterbricht künftige Ausführungen vorübergehend und behält die Jobdefinition bei. Mit resume wird der Job fortgesetzt."
            ),
            locale(
                "es-ES",
                summary = "Pausa una tarea programada sin eliminarla.",
                help = "Suspende temporalmente las próximas ejecuciones y conserva la definición de la tarea. Usa resume para reactivarla."
            ),
            locale(
                "hi-IN",
                summary = "शेड्यूल किए गए काम को हटाए बिना रोकें।",
                help = "काम की परिभाषा बनाए रखते हुए उसके आगे के निष्पादन अस्थायी रूप से रोकें। फिर शुरू करने के लिए resume उपयोग करें।"
            ),
            locale(
                "ar-SA",
                summary = "أوقف مهمة مجدولة مؤقتًا دون حذفها.",
                help = "علّق عمليات التنفيذ القادمة مؤقتًا مع الاحتفاظ بتعريف المهمة. استخدم resume لاستئنافها."
            ),
            locale(
                "pt-BR",
                summary = "Pause uma tarefa agendada sem excluí-la.",
                help = "Suspenda temporariamente as próximas execuções sem apagar a definição da tarefa. Use resume para reativá-la."
            )
        )
    )]
    async fn invoke_pause(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
        args: CronJobControlToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_pause",
            serde_json::to_value(args).map_err(|err| PluginError::invalid_params_error(&err))?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate, scheduler),
        summary = "Resume one paused scheduled job.",
        help = "Re-enable a job that was paused so its future firings happen again.",
        translations(
            locale(
                "zh-CN",
                summary = "恢复一条已暂停的计划任务。",
                help = "重新启用已暂停的任务，让它之后再次按计划触发。"
            ),
            locale(
                "zh-TW",
                summary = "恢復一筆已暫停的排程工作。",
                help = "重新啟用已暫停的工作，讓它之後再次依排程觸發。"
            ),
            locale(
                "ja-JP",
                summary = "一時停止中のスケジュールを再開します。",
                help = "一時停止したジョブを再び有効にし、今後の予定を実行します。"
            ),
            locale(
                "ko-KR",
                summary = "일시 중지된 예약 작업을 다시 시작합니다.",
                help = "중지했던 작업을 다시 활성화해 이후 예약된 실행이 이루어지게 합니다."
            ),
            locale(
                "fr-FR",
                summary = "Reprendre une tâche planifiée en pause.",
                help = "Réactive une tâche suspendue pour que ses prochains déclenchements aient lieu."
            ),
            locale(
                "de-DE",
                summary = "Einen pausierten Zeitplan fortsetzen.",
                help = "Aktiviert einen pausierten Job erneut, damit seine künftigen Ausführungen wieder stattfinden."
            ),
            locale(
                "es-ES",
                summary = "Reanuda una tarea programada en pausa.",
                help = "Vuelve a activar una tarea pausada para que se ejecuten sus próximos avisos."
            ),
            locale(
                "hi-IN",
                summary = "रुके हुए शेड्यूल किए गए काम को फिर शुरू करें।",
                help = "रुके हुए काम को दोबारा सक्षम करें ताकि उसके आगे के निष्पादन फिर से हों।"
            ),
            locale(
                "ar-SA",
                summary = "استأنف مهمة مجدولة متوقفة مؤقتًا.",
                help = "أعد تفعيل المهمة المتوقفة لتعود عمليات التنفيذ القادمة في مواعيدها."
            ),
            locale(
                "pt-BR",
                summary = "Retome uma tarefa agendada que está pausada.",
                help = "Reative a tarefa pausada para que os próximos disparos voltem a acontecer."
            )
        )
    )]
    async fn invoke_resume(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
        args: CronJobControlToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_resume",
            serde_json::to_value(args).map_err(|err| PluginError::invalid_params_error(&err))?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(query, scheduler, read_only),
        summary = "Inspect bounded persisted delivery history for scheduled jobs.",
        help = "Read the bounded delivery history (fire times, outcome, last error) for scheduled jobs. Never poll this waiting for a job to fire — the firing itself appends its prompt to the session and wakes you.",
        translations(
            locale(
                "zh-CN",
                summary = "查看计划任务保留的有限投递记录。",
                help = "读取计划任务的有限投递历史，包括触发时间、结果和最近错误。不要反复查询等待任务触发；任务触发时会自行把提示加入会话并唤醒你。"
            ),
            locale(
                "zh-TW",
                summary = "查看排程工作保留的有限投遞紀錄。",
                help = "讀取排程工作的有限投遞歷史，包括觸發時間、結果與最近錯誤。不要反覆查詢等待工作觸發；工作觸發時會自行將提示加入工作階段並喚醒你。"
            ),
            locale(
                "ja-JP",
                summary = "スケジュール実行の保存済み履歴を確認します。",
                help = "実行時刻、結果、直近のエラーを含む、保存された範囲内の配信履歴を読み取ります。実行を待つためにポーリングしないでください。予定が実行されるとプロンプトがセッションに追加され、モデルを起動します。"
            ),
            locale(
                "ko-KR",
                summary = "예약 작업의 저장된 실행 기록을 확인합니다.",
                help = "실행 시각, 결과, 최근 오류가 포함된 제한된 전달 기록을 읽습니다. 작업 실행을 기다리며 폴링하지 마세요. 예약이 실행되면 프롬프트가 세션에 추가되고 모델을 깨웁니다."
            ),
            locale(
                "fr-FR",
                summary = "Consulter l’historique limité des livraisons planifiées.",
                help = "Lit l’historique conservé des déclenchements (heure, résultat, dernière erreur). Ne faites jamais de sondage en attendant l’exécution : le déclenchement ajoute lui-même son prompt à la session et réveille le modèle."
            ),
            locale(
                "de-DE",
                summary = "Den gespeicherten Ausführungsverlauf geplanter Jobs prüfen.",
                help = "Liest den begrenzten Verlauf mit Ausführungszeitpunkten, Ergebnissen und letztem Fehler. Fragen Sie nicht wiederholt ab, um auf einen Job zu warten: Bei der Ausführung wird der Prompt an die Sitzung angehängt und weckt das Modell."
            ),
            locale(
                "es-ES",
                summary = "Consulta el historial guardado de ejecuciones programadas.",
                help = "Lee el historial limitado de entregas, con horas de ejecución, resultado y último error. No consultes repetidamente esperando a que se ejecute una tarea: al dispararse, añadirá su aviso a la sesión y despertará al modelo."
            ),
            locale(
                "hi-IN",
                summary = "शेड्यूल किए गए काम का सीमित सहेजा गया इतिहास देखें।",
                help = "ट्रिगर समय, नतीजे और हाल की त्रुटि सहित सीमित डिलीवरी इतिहास पढ़ें। काम चलने की प्रतीक्षा में पोल न करें—ट्रिगर होने पर उसका प्रॉम्प्ट अपने-आप सत्र में जुड़कर मॉडल को जगाता है।"
            ),
            locale(
                "ar-SA",
                summary = "اعرض سجل التسليم المحفوظ والمحدود للمهام المجدولة.",
                help = "اقرأ سجلًا محدودًا للتنفيذ يتضمن الأوقات والنتائج وآخر خطأ. لا تستطلع السجل انتظارًا للتنفيذ؛ فعند موعد المهمة يُضاف طلبها إلى الجلسة ويوقظ النموذج تلقائيًا."
            ),
            locale(
                "pt-BR",
                summary = "Consulte o histórico limitado de entregas das tarefas agendadas.",
                help = "Leia o histórico limitado de execuções, com horários, resultados e o erro mais recente. Não consulte repetidamente à espera de uma tarefa: quando ela disparar, o prompt será anexado à sessão e despertará o modelo."
            )
        )
    )]
    async fn invoke_history(
        &self,
        context: &agena_plugin_host::sdk::ToolInvokeContext<'_>,
        args: CronHistoryToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            "cron_history",
            serde_json::to_value(args).map_err(|err| PluginError::invalid_params_error(&err))?,
            context.session_id,
            context.call_id,
        )
    }
}
