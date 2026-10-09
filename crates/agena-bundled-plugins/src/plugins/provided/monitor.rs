//! `agena.monitor`: WebSocket event subscriptions. Command-output listeners
//! belong to agena.shell.watch and share Shell process management.

use crate::part::{MonitorToolInput, MonitorWsInput};
use crate::plugins::provided::router;
use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const MONITOR_PLUGIN_ID: &str = "agena.monitor";

pub(crate) struct MonitorPlugin;

pub(crate) fn new_plugin() -> MonitorPlugin {
    MonitorPlugin
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("description"),
    minimum("timeout_ms", 1),
    maximum("timeout_ms", 3600000)
)]
#[serde(deny_unknown_fields)]
pub(crate) struct MonitorStartInput {
    #[input(nested_shape)]
    ws: MonitorWsInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("monitor_id"), non_empty("monitor_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct MonitorStopInput {
    monitor_id: String,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "monitor",
    version = env!("CARGO_PKG_VERSION"),
    summary = "WebSocket event subscriptions with background notifications. Use shell.watch for local command-output monitoring.",
    translations(
        locale("zh-CN", summary = "订阅 WebSocket 事件并在后台通知；监控本地命令输出请用 shell.watch。"),
        locale("zh-TW", summary = "訂閱 WebSocket 事件並在背景通知；監看本機命令輸出請使用 shell.watch。"),
        locale("ja-JP", summary = "WebSocket イベントを購読してバックグラウンドで通知します。ローカルコマンドの出力監視には shell.watch を使います。"),
        locale("ko-KR", summary = "WebSocket 이벤트를 구독해 백그라운드에서 알립니다. 로컬 명령 출력 감시에는 shell.watch를 사용합니다."),
        locale("fr-FR", summary = "S’abonner aux événements WebSocket avec des notifications en arrière-plan. Utiliser shell.watch pour surveiller les commandes locales."),
        locale("de-DE", summary = "WebSocket-Ereignisse abonnieren und im Hintergrund melden. Für lokale Befehlsausgaben shell.watch verwenden."),
        locale("es-ES", summary = "Se suscribe a eventos WebSocket y envía avisos en segundo plano. Usa shell.watch para supervisar comandos locales."),
        locale("hi-IN", summary = "WebSocket इवेंट की सदस्यता लेकर बैकग्राउंड में सूचना दें। स्थानीय कमांड आउटपुट देखने के लिए shell.watch का उपयोग करें।"),
        locale("ar-SA", summary = "اشترك في أحداث WebSocket لتلقي إشعارات في الخلفية. استخدم shell.watch لمراقبة مخرجات الأوامر المحلية."),
        locale("pt-BR", summary = "Assine eventos WebSocket com notificações em segundo plano. Use shell.watch para acompanhar a saída de comandos locais.")
    ),
)]
impl MonitorPlugin {
    #[tool(
        tags(execute, network, mutate),
        summary = "Subscribe to WebSocket text events and notify the AI in bounded batches.",
        help = "Start a background WebSocket subscription using ws.url and optional protocols. This tool does not execute local commands; use shell.watch for command-output events. The endpoint is checked as a network effect. Return monitor_id and continue working; text events arrive as bounded system_notification batches at most once per second, followed by a final notification when the subscription ends. Do not poll or sleep to wait. timeout_ms defaults to 300000 and is enforced across connection establishment and the feed lifetime (maximum 3600000). The subscription also ends on disconnect, explicit stop, cancellation or session end. Use monitor.stop for cleanup.",
        translations(
            locale(
                "zh-CN",
                summary = "订阅 WebSocket 文本事件，并分批通知 AI。",
                help = "使用 ws.url 和可选协议启动后台 WebSocket 订阅。此工具不会运行本地命令；监听命令输出请用 shell.watch。目标地址会按网络操作检查。返回 monitor_id 后继续工作；文本事件会以有上限的 system_notification 批次送达，最多每秒一次，订阅结束时还会收到最终通知。不要轮询或休眠等待。timeout_ms 默认 300000，覆盖连接建立和整个订阅过程，最大为 3600000。断开连接、主动停止、取消或会话结束时订阅都会结束；请用 monitor.stop 清理。"
            ),
            locale(
                "zh-TW",
                summary = "訂閱 WebSocket 文字事件，並分批通知 AI。",
                help = "使用 ws.url 和可選協定啟動背景 WebSocket 訂閱。此工具不會執行本機命令；監看命令輸出請使用 shell.watch。目標網址會依網路操作檢查。回傳 monitor_id 後即可繼續工作；文字事件會以有上限的 system_notification 批次送達，最多每秒一次，訂閱結束時另有最終通知。不要輪詢或休眠等待。timeout_ms 預設為 300000，涵蓋連線建立與整個訂閱期間，最大值為 3600000。連線中斷、主動停止、取消或工作階段結束時訂閱都會結束；請使用 monitor.stop 清理。"
            ),
            locale(
                "ja-JP",
                summary = "WebSocket のテキストイベントを購読し、まとめて AI に通知します。",
                help = "ws.url と任意のプロトコルを指定して、バックグラウンドで WebSocket を購読します。ローカルコマンドは実行しません。コマンド出力の監視には shell.watch を使ってください。接続先はネットワーク操作として確認されます。monitor_id を受け取ったら作業を続けてください。テキストイベントは上限付きの system_notification として最大 1 秒に 1 回届き、購読終了時にも最終通知が届きます。待つためにポーリングしたり sleep したりしないでください。timeout_ms は既定で 300000、接続確立から購読中まで適用され、最大値は 3600000 です。切断、明示的な停止、キャンセル、セッション終了でも購読は終了します。後片付けには monitor.stop を使ってください。"
            ),
            locale(
                "ko-KR",
                summary = "WebSocket 텍스트 이벤트를 구독해 묶음으로 AI에 알립니다.",
                help = "ws.url과 선택적 프로토콜로 백그라운드 WebSocket 구독을 시작합니다. 이 도구는 로컬 명령을 실행하지 않습니다. 명령 출력 이벤트는 shell.watch를 사용하세요. 엔드포인트는 네트워크 작업으로 검사됩니다. monitor_id를 반환받으면 계속 작업하세요. 텍스트 이벤트는 크기가 제한된 system_notification 묶음으로 최대 초당 한 번 전달되고 구독 종료 시 최종 알림도 옵니다. 기다리려고 폴링하거나 sleep하지 마세요. timeout_ms는 기본 300000이며 연결 수립부터 구독 종료까지 적용되고 최댓값은 3600000입니다. 연결 끊김, 직접 중지, 취소, 세션 종료 시에도 끝납니다. 정리에는 monitor.stop을 사용하세요."
            ),
            locale(
                "fr-FR",
                summary = "S’abonner aux événements texte WebSocket et les notifier par lots limités.",
                help = "Démarrez un abonnement WebSocket en arrière-plan avec ws.url et des protocoles facultatifs. Cet outil n’exécute pas de commandes locales : utilisez shell.watch pour les sorties de commande. L’adresse est vérifiée comme effet réseau. Récupérez monitor_id et poursuivez le travail ; les événements texte arrivent dans des system_notification de taille limitée, au plus une fois par seconde, puis une dernière notification signale la fin. Ne faites pas de polling et ne dormez pas pour attendre. timeout_ms vaut 300000 par défaut et couvre la connexion ainsi que tout l’abonnement (maximum 3600000). L’abonnement prend aussi fin à la déconnexion, à l’arrêt demandé, à l’annulation ou à la fin de session. Utilisez monitor.stop pour le nettoyer."
            ),
            locale(
                "de-DE",
                summary = "WebSocket-Textereignisse abonnieren und in begrenzten Gruppen melden.",
                help = "Starten Sie mit ws.url und optionalen Protokollen ein WebSocket-Abo im Hintergrund. Dieses Tool führt keine lokalen Befehle aus; verwenden Sie für Befehlsausgaben shell.watch. Der Endpunkt wird als Netzwerkzugriff geprüft. Übernehmen Sie monitor_id und arbeiten Sie weiter. Texte treffen in begrenzten system_notification-Gruppen höchstens einmal pro Sekunde ein; beim Ende folgt eine Abschlussmeldung. Warten Sie nicht durch Polling oder sleep. timeout_ms beträgt standardmäßig 300000 und gilt vom Verbindungsaufbau bis zum Ende des Feeds (höchstens 3600000). Bei Verbindungsabbruch, explizitem Stopp, Abbruch oder Sitzungsende endet das Abo ebenfalls. Verwenden Sie monitor.stop zum Aufräumen."
            ),
            locale(
                "es-ES",
                summary = "Suscríbete a eventos de texto WebSocket y notifícalos en lotes limitados.",
                help = "Inicia una suscripción WebSocket en segundo plano con ws.url y protocolos opcionales. Esta herramienta no ejecuta comandos locales; usa shell.watch para eventos de salida de comandos. El destino se comprueba como operación de red. Guarda monitor_id y sigue trabajando. Los eventos llegan en lotes limitados de system_notification como máximo una vez por segundo, seguidos de un aviso final cuando termina la suscripción. No consultes en bucle ni duermas para esperar. timeout_ms es 300000 por defecto y cubre la conexión y toda la suscripción (máximo 3600000). También termina al desconectarse, al detenerla expresamente, al cancelarla o al finalizar la sesión. Usa monitor.stop para limpiar."
            ),
            locale(
                "hi-IN",
                summary = "WebSocket टेक्स्ट इवेंट सुनें और सीमित बैच में AI को सूचना दें।",
                help = "ws.url और वैकल्पिक प्रोटोकॉल से बैकग्राउंड WebSocket सदस्यता शुरू करें। यह टूल स्थानीय कमांड नहीं चलाता; कमांड आउटपुट इवेंट के लिए shell.watch उपयोग करें। एंडपॉइंट की नेटवर्क कार्रवाई के रूप में जाँच होती है। monitor_id लेकर अपना काम जारी रखें। टेक्स्ट इवेंट सीमित system_notification बैच में अधिकतम हर सेकंड एक बार आते हैं; सदस्यता खत्म होने पर अंतिम सूचना भी मिलती है। इंतज़ार के लिए पोल या sleep न करें। timeout_ms का डिफ़ॉल्ट 300000 है और यह कनेक्शन बनने से लेकर पूरी सदस्यता तक लागू होता है (अधिकतम 3600000)। डिस्कनेक्ट, स्पष्ट रोक, रद्दीकरण या सत्र समाप्त होने पर सदस्यता खत्म हो जाती है। सफ़ाई के लिए monitor.stop उपयोग करें।"
            ),
            locale(
                "ar-SA",
                summary = "اشترك في أحداث WebSocket النصية وأبلغ عنها على دفعات محدودة.",
                help = "ابدأ اشتراك WebSocket في الخلفية باستخدام ws.url وبروتوكولات اختيارية. لا تنفذ هذه الأداة أوامر محلية؛ استخدم shell.watch لأحداث مخرجات الأوامر. يُفحص العنوان باعتباره اتصالًا بالشبكة. احتفظ بـ monitor_id وتابع العمل؛ تصل الأحداث النصية في دفعات system_notification محدودة، بحد أقصى مرة في الثانية، ثم يصل إشعار أخير عند انتهاء الاشتراك. لا تستطلع الخدمة ولا تنتظر عبر sleep. القيمة الافتراضية لـ timeout_ms هي 300000 وتشمل إنشاء الاتصال وعمر التدفق كله، بحد أقصى 3600000. ينتهي الاشتراك أيضًا عند الانقطاع أو الإيقاف الصريح أو الإلغاء أو نهاية الجلسة. استخدم monitor.stop للتنظيف."
            ),
            locale(
                "pt-BR",
                summary = "Assine eventos de texto WebSocket e notifique a IA em lotes limitados.",
                help = "Inicie uma assinatura WebSocket em segundo plano usando ws.url e protocolos opcionais. Esta ferramenta não executa comandos locais; use shell.watch para eventos de saída de comandos. O destino é verificado como acesso de rede. Guarde monitor_id e continue trabalhando. Os eventos de texto chegam em lotes limitados de system_notification, no máximo uma vez por segundo, seguidos de um aviso final quando a assinatura termina. Não consulte em loop nem use sleep para esperar. timeout_ms é 300000 por padrão e vale desde a conexão até o fim do fluxo (máximo 3600000). A assinatura também termina em caso de desconexão, parada explícita, cancelamento ou fim da sessão. Use monitor.stop para limpar."
            )
        )
    )]
    async fn invoke_start(
        &self,
        context: &ToolInvokeContext<'_>,
        args: MonitorStartInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "monitor",
            json_input(MonitorToolInput::Start {
                ws: args.ws,
                timeout_ms: args.timeout_ms,
                description: args.description,
            })?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate, network),
        summary = "Stop one WebSocket subscription.",
        translations(
            locale("zh-CN", summary = "停止一条 WebSocket 订阅。"),
            locale("zh-TW", summary = "停止一筆 WebSocket 訂閱。"),
            locale("ja-JP", summary = "WebSocket の購読を停止します。"),
            locale("ko-KR", summary = "WebSocket 구독을 중지합니다."),
            locale("fr-FR", summary = "Arrêter un abonnement WebSocket."),
            locale("de-DE", summary = "Ein WebSocket-Abo beenden."),
            locale("es-ES", summary = "Detiene una suscripción WebSocket."),
            locale("hi-IN", summary = "WebSocket सदस्यता रोकें।"),
            locale("ar-SA", summary = "أوقف اشتراك WebSocket."),
            locale("pt-BR", summary = "Encerre uma assinatura WebSocket.")
        )
    )]
    async fn invoke_stop(
        &self,
        context: &ToolInvokeContext<'_>,
        args: MonitorStopInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "monitor",
            json_input(MonitorToolInput::Stop {
                monitor_id: args.monitor_id,
            })?,
            context.session_id,
            context.call_id,
        )
    }
}

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params_error(&err))
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::Plugin;

    use super::MonitorPlugin;

    #[test]
    fn manifest_exposes_monitor_tools_under_the_monitor_plugin() {
        let manifest = MonitorPlugin.manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "monitor");
        assert_eq!(tool_names, ["start", "stop"]);
        let start = manifest
            .tools
            .iter()
            .find(|tool| tool.name == "start")
            .expect("monitor.start manifest");
        let schema = serde_json::to_string(&start.input_schema()).expect("serialize schema");
        assert!(!schema.contains("\"command\""));
        assert!(schema.contains("ws"));
        assert!(schema.contains("timeout_ms"));
    }
}
