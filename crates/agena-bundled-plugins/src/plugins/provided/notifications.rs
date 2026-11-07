//! `agena.notifications`: shared attention policy and terminal activity display.
//!
//! Canonical kernel execution and interaction facts produce persisted
//! `host.notify` intents, shared by the terminal and browser delivery adapters.
//! The observer lives with the plugin, independently of connected pages, and
//! stops when the plugin shuts down. A separate read-only kernel observer
//! projects terminal activity into every runtime snapshot's display registry.
//! Lifecycle hooks provide a fallback only when no canonical source is supplied.
//!
//! The plugin exposes no AI tools. Terminal OSC capability detection and
//! browser Web Push delivery remain responsibilities of the respective adapters.

use std::sync::{Arc, Mutex, RwLock};

mod activity;
mod attention;
pub use attention::NotificationSource;

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{
    HostClient, HostDisplayContributeRequest, HostDisplayRemoveRequest,
};
use agena_plugin_host::sdk::{
    AgentStopInput, AgentStopPatch, ContributionKind, InitContext, InitOutcome,
    PluginDisplayContent, PluginDisplayContribution, PostRunInput, PreRunInput,
    Result as SdkResult, RunStatus, SessionEndInput, SessionStartInput, SessionStartPatch,
    UserPromptSubmitInput, UserPromptSubmitPatch,
};

pub(crate) const NOTIFICATIONS_PLUGIN_ID: &str = "agena.notifications";

/// Display contribution id published by this plugin. The TUI reads exactly
/// this id to reconstruct the terminal activity state.
pub(crate) const ACTIVITY_CONTRIBUTION_ID: &str = "agena.notifications.terminal.activity";

/// Display priority for the activity contribution. One-shot attention uses
/// the unified `host.notify` entry; the activity state rides the declarative
/// display channel (`host.display_contribute`).
pub(crate) const ACTIVITY_CONTRIBUTION_PRIORITY: i32 = i32::MAX - 1;

/// The activity phase projected to the terminal title.
///
/// Canonical session facts distinguish active work, pending user interaction
/// and the latest failed outcome across all visible sessions. Standalone plugin
/// hosts without a notification source retain the lifecycle-hook fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TerminalActivity {
    Idle,
    Running,
    Blocked,
}

pub(crate) struct NotificationsPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
    activity: Arc<RwLock<TerminalActivity>>,
    notification_source: Option<NotificationSource>,
    notification_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    activity_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

async fn publish_activity(host: &dyn HostClient, activity: TerminalActivity) -> SdkResult<()> {
    let value = match activity {
        TerminalActivity::Idle => "idle",
        TerminalActivity::Running => "running",
        TerminalActivity::Blocked => "blocked",
    };
    host.display_contribute(HostDisplayContributeRequest {
        contribution: PluginDisplayContribution {
            id: ACTIVITY_CONTRIBUTION_ID.to_owned(),
            kind: ContributionKind::TerminalActivity,
            priority: ACTIVITY_CONTRIBUTION_PRIORITY,
            content: PluginDisplayContent::TerminalActivity {
                value: value.to_owned(),
            },
        },
    })
    .await
    .map(|_| ())
}

fn recover_read<'a, T>(lock: &'a RwLock<T>, context: &str) -> std::sync::RwLockReadGuard<'a, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(error) => {
            tracing::error!(
                operation = context,
                error = %error,
                "recovering poisoned notifications plugin read lock"
            );
            error.into_inner()
        }
    }
}

fn recover_write<'a, T>(lock: &'a RwLock<T>, context: &str) -> std::sync::RwLockWriteGuard<'a, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(error) => {
            tracing::error!(
                operation = context,
                error = %error,
                "recovering poisoned notifications plugin write lock"
            );
            error.into_inner()
        }
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "notifications",
    version = env!("CARGO_PKG_VERSION"),
    summary = "AI completion, question and approval notifications for browsers and terminals, with terminal window titles.",
    translations(
        locale("zh-CN", summary = "统一浏览器和终端的 AI 完成、提问与审批通知，并同步终端窗口标题。"),
        locale("zh-TW", summary = "統一瀏覽器和終端機的 AI 完成、提問與審批通知，並同步終端機視窗標題。"),
        locale("ja-JP", summary = "ブラウザーとターミナルで AI の完了・質問・承認通知を共有し、ターミナルのウィンドウタイトルを同期します。"),
        locale("ko-KR", summary = "브라우저와 터미널에서 AI 완료, 질문 및 승인 알림을 통합하고 터미널 창 제목을 동기화합니다."),
        locale("fr-FR", summary = "Notifications de fin de réponse, de question et d’approbation de l’IA pour les navigateurs et terminaux, avec synchronisation du titre du terminal."),
        locale("de-DE", summary = "Benachrichtigungen zu KI-Abschlüssen, Fragen und Genehmigungen für Browser und Terminals sowie Synchronisierung des Terminalfenstertitels."),
        locale("es-ES", summary = "Notificaciones de respuestas completadas, preguntas y aprobaciones de la IA para navegadores y terminales, con sincronización del título del terminal."),
        locale("hi-IN", summary = "ब्राउज़र और टर्मिनल के लिए AI के कार्य पूर्ण होने, प्रश्नों और अनुमोदनों की सूचनाएँ तथा टर्मिनल विंडो शीर्षक का समन्वयन।"),
        locale("ar-SA", summary = "إشعارات اكتمال ردود الذكاء الاصطناعي والأسئلة وطلبات الموافقة في المتصفح والطرفية، مع مزامنة عنوان نافذة الطرفية."),
        locale("pt-BR", summary = "Notificações de respostas concluídas, perguntas e aprovações da IA para navegadores e terminais, com sincronização do título do terminal.")
    ),
)]
impl NotificationsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
            activity: Arc::new(RwLock::new(TerminalActivity::Idle)),
            notification_source: None,
            notification_task: Mutex::new(None),
            activity_task: Mutex::new(None),
        }
    }

    pub(crate) fn with_notifications(mut self, source: Option<NotificationSource>) -> Self {
        self.notification_source = source;
        self
    }

    async fn stop_notifications(&self) {
        // Take ownership before awaiting; neither std mutex is held across the
        // join. Both workers must stop before removing the display contribution.
        let tasks: Vec<_> = [&self.notification_task, &self.activity_task]
            .into_iter()
            .filter_map(|slot| {
                slot.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
            })
            .collect();
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            if let Err(error) = task.await
                && !error.is_cancelled()
            {
                tracing::warn!(%error, "notifications worker failed during shutdown");
            }
        }
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        recover_read(&self.host, "read notifications plugin host")
            .clone()
            .ok_or_else(|| PluginError::internal("notifications plugin invoked before init"))
    }

    /// Record the current activity
    /// and publish it through the declarative display channel so the TUI can
    /// render the terminal title. Best-effort: a missing host or a failed
    /// write is non-fatal because the TUI retains a local fallback.
    async fn set_and_publish(&self, activity: TerminalActivity) {
        // A callback belongs to one run/snapshot, not the global activity.
        // In canonical mode even session.start/end and a retired run.post must
        // not clear another session's work or override a newly loaded host.
        if self.notification_source.is_some() {
            return;
        }
        *recover_write(&self.activity, "update terminal activity") = activity;
        self.publish_display().await;
    }

    async fn publish_display(&self) {
        let host = match self.host() {
            Ok(host) => host,
            Err(error) => {
                tracing::warn!(
                    diagnostic = %error.diagnostic.message,
                    "terminal display publication skipped because the host is unavailable"
                );
                return;
            }
        };
        let activity = *recover_read(&self.activity, "read terminal activity");
        if let Err(error) = publish_activity(host.as_ref(), activity).await {
            tracing::warn!(
                diagnostic = %error.diagnostic.message,
                "failed to publish terminal activity display"
            );
        }
    }

    /// Clear the display contribution on shutdown.
    async fn clear_display(&self) -> SdkResult<()> {
        let host = self.host()?;
        host.display_remove(HostDisplayRemoveRequest {
            contribution_id: ACTIVITY_CONTRIBUTION_ID.to_owned(),
        })
        .await
        .map(|_| ())
    }

    #[hook(
        init,
        summary = "Start notification services",
        help = "Initializes terminal activity and notification tracking through the host.",
        translations(
            locale(
                "zh-CN",
                summary = "初始化通知服务",
                help = "准备通知服务功能并连接所需的 Host 服务。"
            ),
            locale(
                "zh-TW",
                summary = "初始化通知服務",
                help = "準備通知服務功能並連接所需的 Host 服務。"
            ),
            locale(
                "fr-FR",
                summary = "Initialiser les services de notification",
                help = "Prépare les services de notification et connecte les services hôtes nécessaires."
            ),
            locale(
                "es-ES",
                summary = "Inicializar los servicios de notificación",
                help = "Prepara los servicios de notificación y conecta los servicios del host necesarios."
            ),
            locale(
                "pt-BR",
                summary = "Inicializar os serviços de notificação",
                help = "Prepara os serviços de notificação e conecta os serviços necessários do host."
            ),
            locale(
                "de-DE",
                summary = "Die Benachrichtigungsdienste initialisieren",
                help = "Bereitet die Benachrichtigungsdienste vor und verbindet die erforderlichen Host-Dienste."
            ),
            locale(
                "ja-JP",
                summary = "通知サービスを初期化",
                help = "必要なホストサービスに接続し、通知サービスを利用できるようにします。"
            ),
            locale(
                "ko-KR",
                summary = "알림 서비스 초기화",
                help = "필요한 호스트 서비스에 연결하고 알림 서비스 기능을 준비합니다."
            ),
            locale(
                "hi-IN",
                summary = "सूचना सेवाएँ प्रारंभ करें",
                help = "“सूचना सेवाएँ” के लिए ज़रूरी होस्ट सेवाओं से जुड़कर संबंधित कार्यक्षमता तैयार करता है।"
            ),
            locale(
                "ar-SA",
                summary = "تهيئة خدمات الإشعارات",
                help = "يُعِدّ خدمات الإشعارات ويربط خدمات المضيف اللازمة."
            )
        )
    )]
    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.stop_notifications().await;
        *recover_write(&self.host, "initialize notifications plugin host") = Some(host.clone());
        if let Some(source) = self.notification_source.clone() {
            let task = activity::start(source.reader.clone(), host.clone())
                .await
                .map_err(|error| PluginError::internal(error.to_string()))?;
            *self
                .activity_task
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(task);
            match attention::start(source, host).await {
                Ok(task) => {
                    *self
                        .notification_task
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(task);
                }
                Err(error) => {
                    self.stop_notifications().await;
                    let _ = self.clear_display().await;
                    return Err(PluginError::internal(error.to_string()));
                }
            }
        }
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[hook(
        shutdown,
        summary = "Stop notification services",
        help = "Stops notification tracking and clears terminal activity displays.",
        translations(
            locale(
                "zh-CN",
                summary = "停止通知服务",
                help = "停止通知追踪，并清除终端中的活动状态显示。"
            ),
            locale(
                "zh-TW",
                summary = "停止通知服務",
                help = "停止通知追蹤，並清除終端機中的活動狀態顯示。"
            ),
            locale(
                "fr-FR",
                summary = "Arrêter les services de notification",
                help = "Arrête le suivi des notifications et efface l’indicateur d’activité du terminal."
            ),
            locale(
                "es-ES",
                summary = "Detener los servicios de notificación",
                help = "Detiene el seguimiento de notificaciones y limpia el indicador de actividad del terminal."
            ),
            locale(
                "pt-BR",
                summary = "Encerrar serviços de notificação",
                help = "Interrompe o acompanhamento de notificações e limpa o indicador de atividade do terminal."
            ),
            locale(
                "de-DE",
                summary = "Benachrichtigungsdienste beenden",
                help = "Beendet die Benachrichtigungsverfolgung und setzt die Aktivitätsanzeige im Terminal zurück."
            ),
            locale(
                "ja-JP",
                summary = "通知サービスを停止",
                help = "通知の追跡を停止し、ターミナルのアクティビティ表示を消去します。"
            ),
            locale(
                "ko-KR",
                summary = "알림 서비스 중지",
                help = "알림 추적을 중지하고 터미널의 활동 표시를 지웁니다."
            ),
            locale(
                "hi-IN",
                summary = "सूचना सेवाएँ बंद करें",
                help = "सूचनाओं की निगरानी रोकता है और टर्मिनल से गतिविधि संकेतक हटाता है।"
            ),
            locale(
                "ar-SA",
                summary = "إيقاف خدمات الإشعارات",
                help = "يوقف تتبّع الإشعارات ويمسح مؤشر النشاط من الطرفية."
            )
        )
    )]
    async fn shutdown(&self) -> SdkResult<()> {
        self.stop_notifications().await;
        self.clear_display().await
    }

    #[hook(
        run.pre,
        summary = "Show running status",
        help = "Updates the activity indicator when an Agent execution begins.",
        translations(
        locale("zh-CN", summary = "显示运行状态", help = "当 Agent 开始执行任务时更新活动状态指示器。"),
        locale("zh-TW", summary = "顯示執行狀態", help = "Agent 開始執行工作時更新活動狀態指示器。"),
        locale("fr-FR", summary = "Afficher l’activité", help = "Met à jour l’indicateur d’activité au début de l’exécution."),
        locale("es-ES", summary = "Mostrar la actividad en curso", help = "Actualiza el indicador de actividad al comenzar la ejecución del agente."),
        locale("pt-BR", summary = "Exibir atividade em execução", help = "Atualiza o indicador de atividade quando o agente começa a executar."),
        locale("de-DE", summary = "Aktiven Ausführungsstatus anzeigen", help = "Aktualisiert die Aktivitätsanzeige, sobald die Ausführung des Agenten beginnt."),
        locale("ja-JP", summary = "実行中の状態を表示", help = "エージェントの実行開始時にアクティビティ表示を更新します。"),
        locale("ko-KR", summary = "실행 중 상태 표시", help = "에이전트 실행이 시작되면 활동 표시를 업데이트합니다."),
        locale("hi-IN", summary = "चल रही गतिविधि दिखाएँ", help = "एजेंट का निष्पादन शुरू होने पर गतिविधि संकेतक अपडेट करता है।"),
        locale("ar-SA", summary = "عرض حالة التنفيذ الجارية", help = "يحدّث مؤشر النشاط عند بدء تنفيذ الوكيل.")
    )
    )]
    async fn pre_run(&self, _input: PreRunInput) -> SdkResult<()> {
        self.set_and_publish(TerminalActivity::Running).await;
        Ok(())
    }

    #[hook(run.post, summary = "Update execution status", help = "Updates terminal activity when a session execution succeeds, fails or is cancelled.",
    translations(
        locale("zh-CN", summary = "更新执行状态", help = "根据会话执行成功、失败或取消的结果更新终端活动状态。"),
        locale("zh-TW", summary = "更新執行狀態", help = "依照工作階段執行成功、失敗或取消的結果更新終端機活動狀態。"),
        locale("fr-FR", summary = "Actualiser l’état d’exécution", help = "Met à jour l’activité du terminal lorsque l’exécution réussit, échoue ou est annulée."),
        locale("es-ES", summary = "Actualizar el estado de ejecución", help = "Actualiza la actividad del terminal cuando la ejecución termina, falla o se cancela."),
        locale("pt-BR", summary = "Atualizar o status da execução", help = "Atualiza a atividade do terminal quando a execução é concluída, falha ou é cancelada."),
        locale("de-DE", summary = "Ausführungsstatus aktualisieren", help = "Aktualisiert die Terminalaktivität, wenn eine Ausführung erfolgreich, fehlerhaft oder abgebrochen endet."),
        locale("ja-JP", summary = "実行状態を更新", help = "セッションの実行が成功、失敗、またはキャンセルされた際に表示を更新します。"),
        locale("ko-KR", summary = "실행 상태 업데이트", help = "세션 실행이 성공하거나 실패 또는 취소되면 터미널 활동 상태를 업데이트합니다."),
        locale("hi-IN", summary = "निष्पादन की स्थिति अपडेट करें", help = "सत्र का निष्पादन सफल, विफल या रद्द होने पर टर्मिनल की स्थिति अपडेट करता है।"),
        locale("ar-SA", summary = "تحديث حالة التنفيذ", help = "يحدّث نشاط الطرفية عند نجاح التنفيذ أو فشله أو إلغائه.")
    )
)]
    async fn post_run(&self, input: PostRunInput) -> SdkResult<()> {
        if input.status == RunStatus::Failed {
            self.set_and_publish(TerminalActivity::Blocked).await;
        } else {
            // Attention is emitted from committed kernel facts, not hooks.
            self.set_and_publish(TerminalActivity::Idle).await;
        }
        Ok(())
    }

    #[hook(session.start, summary = "Reset session activity", help = "Clears stale terminal activity when a session starts or resumes.",
    translations(
        locale("zh-CN", summary = "重置会话活动状态", help = "会话启动或恢复时清除过期的终端活动状态。"),
        locale("zh-TW", summary = "重設工作階段活動狀態", help = "工作階段啟動或恢復時清除過期的終端機活動狀態。"),
        locale("fr-FR", summary = "Réinitialiser l’activité de la session", help = "Efface l’état d’activité obsolète à l’ouverture ou à la reprise d’une session."),
        locale("es-ES", summary = "Restablecer la actividad de la sesión", help = "Elimina estados de actividad obsoletos cuando comienza o se reanuda una sesión."),
        locale("pt-BR", summary = "Redefinir a atividade da sessão", help = "Limpa estados de atividade antigos quando uma sessão começa ou é retomada."),
        locale("de-DE", summary = "Sitzungsaktivität zurücksetzen", help = "Entfernt veraltete Aktivitätsanzeigen, wenn eine Sitzung startet oder fortgesetzt wird."),
        locale("ja-JP", summary = "セッションの活動状態をリセット", help = "セッションの開始または再開時に古い活動状態を消去します。"),
        locale("ko-KR", summary = "세션 활동 상태 초기화", help = "세션이 시작되거나 재개될 때 이전 활동 상태를 지웁니다."),
        locale("hi-IN", summary = "सत्र की गतिविधि रीसेट करें", help = "सत्र शुरू या पुनः चालू होने पर पुरानी गतिविधि स्थिति साफ़ करता है।"),
        locale("ar-SA", summary = "إعادة ضبط نشاط الجلسة", help = "يمسح حالة النشاط القديمة عند بدء الجلسة أو استئنافها.")
    )
)]
    async fn session_start(
        &self,
        _input: SessionStartInput,
    ) -> SdkResult<Option<SessionStartPatch>> {
        self.set_and_publish(TerminalActivity::Idle).await;
        Ok(None)
    }

    #[hook(session.end, summary = "Clear finished session activity", help = "Returns the activity indicator to idle when the session ends.",
    translations(
        locale("zh-CN", summary = "清理已结束会话的状态", help = "会话结束时将终端活动指示器恢复为空闲。"),
        locale("zh-TW", summary = "清理已結束工作階段的狀態", help = "工作階段結束時將終端機活動指示器恢復為閒置。"),
        locale("fr-FR", summary = "Effacer l’activité des sessions terminées", help = "Remet l’indicateur d’activité du terminal en veille à la fin de la session."),
        locale("es-ES", summary = "Limpiar la actividad de sesiones finalizadas", help = "Restablece el indicador de actividad del terminal cuando termina una sesión."),
        locale("pt-BR", summary = "Limpar a atividade de sessões encerradas", help = "Retorna o indicador de atividade do terminal ao estado ocioso quando a sessão termina."),
        locale("de-DE", summary = "Aktivität beendeter Sitzungen löschen", help = "Setzt die Aktivitätsanzeige nach dem Sitzungsende auf inaktiv zurück."),
        locale("ja-JP", summary = "終了したセッションの状態を消去", help = "セッション終了時にターミナルの活動表示をアイドル状態に戻します。"),
        locale("ko-KR", summary = "종료된 세션 활동 정리", help = "세션 종료 시 터미널 활동 표시를 유휴 상태로 되돌립니다."),
        locale("hi-IN", summary = "समाप्त सत्र की गतिविधि साफ़ करें", help = "सत्र समाप्त होने पर टर्मिनल संकेतक को निष्क्रिय स्थिति में लौटाता है।"),
        locale("ar-SA", summary = "مسح نشاط الجلسات المنتهية", help = "يعيد مؤشر نشاط الطرفية إلى حالة الخمول عند انتهاء الجلسة.")
    )
)]
    async fn session_end(&self, _input: SessionEndInput) -> SdkResult<()> {
        self.set_and_publish(TerminalActivity::Idle).await;
        Ok(())
    }

    #[hook(prompt.submit, summary = "Show prompt activity", help = "Marks the terminal as busy when a user prompt is submitted.",
    translations(
        locale("zh-CN", summary = "显示提示词处理状态", help = "用户提交提示词时将终端活动指示器设为忙碌。"),
        locale("zh-TW", summary = "顯示提示詞處理狀態", help = "使用者提交提示詞時將終端機活動指示器設為忙碌。"),
        locale("fr-FR", summary = "Signaler le traitement du message", help = "Indique une activité en cours dès qu’un utilisateur envoie un message."),
        locale("es-ES", summary = "Mostrar el procesamiento del mensaje", help = "Marca el terminal como ocupado cuando el usuario envía una solicitud."),
        locale("pt-BR", summary = "Mostrar o processamento da solicitação", help = "Marca o terminal como ocupado quando o usuário envia uma solicitação."),
        locale("de-DE", summary = "Nachrichtenverarbeitung anzeigen", help = "Markiert das Terminal als beschäftigt, sobald der Benutzer eine Anfrage sendet."),
        locale("ja-JP", summary = "入力処理中の状態を表示", help = "ユーザーが入力を送信するとターミナルを処理中として表示します。"),
        locale("ko-KR", summary = "요청 처리 상태 표시", help = "사용자가 요청을 제출하면 터미널을 처리 중 상태로 표시합니다."),
        locale("hi-IN", summary = "अनुरोध का प्रसंस्करण दिखाएँ", help = "उपयोगकर्ता के अनुरोध भेजते ही टर्मिनल को व्यस्त दिखाता है।"),
        locale("ar-SA", summary = "عرض حالة معالجة الطلب", help = "يشير إلى انشغال الطرفية بمجرد إرسال المستخدم طلبًا.")
    )
)]
    async fn user_prompt_submit(
        &self,
        _input: UserPromptSubmitInput,
    ) -> SdkResult<Option<UserPromptSubmitPatch>> {
        // The user submitted a prompt; the run will follow shortly. Mark the
        // state as running so the title reflects it before pre_run fires.
        self.set_and_publish(TerminalActivity::Running).await;
        Ok(None)
    }

    #[hook(agent.stop, summary = "Reflect agent stop outcome", help = "Updates terminal activity according to whether the agent stopped successfully or with an error.",
    translations(
        locale("zh-CN", summary = "反映 Agent 停止结果", help = "根据 Agent 正常停止或发生错误的结果更新终端状态。"),
        locale("zh-TW", summary = "反映 Agent 停止結果", help = "根據 Agent 正常停止或發生錯誤的結果更新終端機狀態。"),
        locale("fr-FR", summary = "Afficher le résultat d’arrêt de l’agent", help = "Actualise l’état du terminal selon que l’agent s’arrête normalement ou avec une erreur."),
        locale("es-ES", summary = "Reflejar el resultado de detención del agente", help = "Actualiza el terminal según si el agente se detuvo correctamente o con un error."),
        locale("pt-BR", summary = "Refletir o resultado da parada do agente", help = "Atualiza o terminal conforme o agente pare normalmente ou com erro."),
        locale("de-DE", summary = "Ergebnis des Agentenstopps anzeigen", help = "Aktualisiert die Terminalanzeige je nachdem, ob der Agent erfolgreich oder mit Fehler stoppt."),
        locale("ja-JP", summary = "エージェントの停止結果を反映", help = "正常停止かエラーによる停止かに応じてターミナルの状態を更新します。"),
        locale("ko-KR", summary = "에이전트 중지 결과 반영", help = "에이전트가 정상 종료되었는지 오류로 중지되었는지에 따라 터미널 상태를 변경합니다."),
        locale("hi-IN", summary = "एजेंट रुकने का परिणाम दिखाएँ", help = "एजेंट सामान्य रूप से रुका या त्रुटि के साथ, उसके अनुसार टर्मिनल स्थिति बदलता है।"),
        locale("ar-SA", summary = "إظهار نتيجة توقف الوكيل", help = "يحدّث حالة الطرفية وفقًا لتوقف الوكيل طبيعيًا أو بسبب خطأ.")
    )
)]
    async fn agent_stop(&self, input: AgentStopInput) -> SdkResult<Option<AgentStopPatch>> {
        if input.run_error.is_some() {
            self.set_and_publish(TerminalActivity::Blocked).await;
        } else {
            self.set_and_publish(TerminalActivity::Idle).await;
        }
        Ok(None)
    }
}

impl Drop for NotificationsPlugin {
    fn drop(&mut self) {
        for slot in [&mut self.notification_task, &mut self.activity_task] {
            if let Some(task) = slot
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                task.abort();
            }
        }
    }
}
