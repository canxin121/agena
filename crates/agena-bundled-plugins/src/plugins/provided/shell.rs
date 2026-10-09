//! `agena.shell`: foreground, background, watched and interactive execution.

use crate::part::{
    ShellLaunchInput, ShellOpenInput, ShellReadInput, ShellSignal, ShellToolInput, ShellWatchInput,
    ShellWriteInput,
};
use crate::plugins::provided::router;
use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput, ToolTag};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const SHELL_PLUGIN_ID: &str = "agena.shell";

pub(crate) struct ShellPlugin;

pub(crate) fn new_plugin() -> ShellPlugin {
    ShellPlugin
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("process_id"),
    non_empty("process_id"),
    minimum("limit", 1),
    maximum("limit", 2000),
    maximum("wait_ms", 30000),
    minimum("max_output_bytes", 1024),
    maximum("max_output_bytes", 16384)
)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessLogsInput {
    process_id: String,
    /// Return events after this cursor; continue with the returned last_seq.
    #[serde(default)]
    since_seq: u64,
    #[serde(default)]
    event_offset: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_output_bytes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default)]
    wait_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("process_id"), non_empty("process_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessStopInput {
    process_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("process_id"),
    non_empty("process_id"),
    minimum("rows", 1),
    maximum("rows", 200),
    minimum("cols", 1),
    maximum("cols", 400)
)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessResizeInput {
    process_id: String,
    rows: u16,
    cols: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("process_id"), non_empty("process_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessSignalInput {
    process_id: String,
    signal: ShellSignal,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "shell",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Shell commands: exec waits, spawn runs in background, watch monitors output events, open provides an interactive terminal/PTY; shared logs and lifecycle controls manage them.",
    translations(
        locale("zh-CN", summary = "Shell 工具：exec 等待命令结束，spawn 在后台运行，watch 监看输出事件，open 打开交互式终端；日志和进程生命周期统一管理。"),
        locale("zh-TW", summary = "Shell 工具：exec 等待命令結束，spawn 在背景執行，watch 監看輸出事件，open 開啟互動式終端；日誌與程序生命週期由同一套工具管理。"),
        locale("ja-JP", summary = "exec は終了まで待機し、spawn はバックグラウンド実行、watch は出力イベントを監視、open は対話型ターミナルを開きます。ログとプロセスのライフサイクルも一元管理します。"),
        locale("ko-KR", summary = "exec는 종료까지 기다리고, spawn은 백그라운드에서 실행하며, watch는 출력 이벤트를 감시하고, open은 대화형 터미널을 엽니다. 로그와 프로세스 수명도 함께 관리합니다."),
        locale("fr-FR", summary = "exec attend la fin de la commande, spawn la lance en arrière-plan, watch suit ses événements de sortie et open ouvre un terminal interactif. Les journaux et le cycle de vie des processus sont gérés au même endroit."),
        locale("de-DE", summary = "exec wartet auf das Befehlsende, spawn startet im Hintergrund, watch verfolgt Ausgabeereignisse und open öffnet ein interaktives Terminal. Protokolle und Prozesslebenszyklus werden gemeinsam verwaltet."),
        locale("es-ES", summary = "exec espera a que termine el comando, spawn lo ejecuta en segundo plano, watch sigue los eventos de salida y open abre un terminal interactivo. Los registros y el ciclo de vida del proceso se gestionan conjuntamente."),
        locale("hi-IN", summary = "exec कमांड पूरा होने तक रुकता है, spawn उसे बैकग्राउंड में चलाता है, watch आउटपुट इवेंट देखता है और open इंटरैक्टिव टर्मिनल खोलता है। लॉग और प्रक्रिया का जीवनचक्र भी यहीं प्रबंधित होता है।"),
        locale("ar-SA", summary = "ينتظر exec انتهاء الأمر، ويشغّله spawn في الخلفية، ويراقب watch أحداث المخرجات، ويفتح open طرفية تفاعلية. كما تُدار السجلات ودورة حياة العملية معًا."),
        locale("pt-BR", summary = "exec aguarda o comando terminar, spawn o executa em segundo plano, watch acompanha os eventos de saída e open abre um terminal interativo. Logs e ciclo de vida dos processos são gerenciados em conjunto.")
    ),
)]
impl ShellPlugin {
    #[tool(
        tags(execute, shell, mutate),
        summary = "Execute a non-interactive shell command and wait for its final output and exit result.",
        translations(
            locale(
                "zh-CN",
                summary = "运行一次性 Shell 命令，等待结束并返回输出和退出状态。"
            ),
            locale(
                "zh-TW",
                summary = "執行一次性 Shell 命令，等待結束並傳回輸出與結束狀態。"
            ),
            locale(
                "ja-JP",
                summary = "シェルコマンドを実行し、終了を待って出力と終了結果を返します。"
            ),
            locale(
                "ko-KR",
                summary = "셸 명령을 실행하고 끝날 때까지 기다린 뒤 출력과 종료 결과를 반환합니다."
            ),
            locale(
                "fr-FR",
                summary = "Exécuter une commande shell jusqu’à son terme et renvoyer sa sortie et son résultat."
            ),
            locale(
                "de-DE",
                summary = "Einen Shell-Befehl bis zum Ende ausführen und Ausgabe sowie Ergebnis zurückgeben."
            ),
            locale(
                "es-ES",
                summary = "Ejecuta un comando de shell hasta que termine y devuelve su salida y resultado."
            ),
            locale(
                "hi-IN",
                summary = "शेल कमांड चलाकर उसके पूरा होने तक प्रतीक्षा करें और आउटपुट व परिणाम लौटाएँ।"
            ),
            locale(
                "ar-SA",
                summary = "نفّذ أمر shell وانتظر اكتماله ثم أعد مخرجاته ونتيجته."
            ),
            locale(
                "pt-BR",
                summary = "Execute um comando shell até o fim e retorne a saída e o resultado."
            )
        ),
        help = "Run a command to completion. Declare actual reads/writes paths and network targets; use empty arrays when none. timeout_ms limits lifetime (default 120000); timeout/cancellation cleans up the owned process tree. max_output_bytes sets the output budget (1024–16384, default 16384), accounting for escaping and reserving lifecycle metadata; previews keep beginning/end and at most 200 lines. Capture is independent of preview: output_resource refers to the same cursor-based content resource used by all Parts. Capture status and retained ranges are independent of process exit and preview limits. Use shell.spawn for background work, shell.watch for readiness/selected notifications, and shell.open for interactive input."
    )]
    async fn invoke_exec(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellLaunchInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Exec {
                shell: args.shell,
                command: Box::new(args.command),
            },
            context,
        )
    }

    #[tool(
        tags(execute, shell, ToolTag::Custom("background".to_owned()), mutate),
        summary = "Spawn a non-interactive shell command in the background; return immediately so the AI can continue, then notify on completion.",
        translations(
            locale("zh-CN", summary = "在后台启动 Shell 命令并立即返回，让 AI 继续工作；任务结束后再通知。"),
            locale("zh-TW", summary = "在背景啟動 Shell 命令並立即返回，讓 AI 繼續工作；結束後再通知。"),
            locale("ja-JP", summary = "シェルコマンドをバックグラウンドで起動してすぐ戻り、完了時に通知します。"),
            locale("ko-KR", summary = "셸 명령을 백그라운드에서 시작하고 즉시 반환해 AI가 계속 작업하게 하며, 완료되면 알립니다."),
            locale("fr-FR", summary = "Lancer une commande shell en arrière-plan, rendre la main immédiatement et notifier sa fin."),
            locale("de-DE", summary = "Einen Shell-Befehl im Hintergrund starten, sofort zurückkehren und nach Abschluss benachrichtigen."),
            locale("es-ES", summary = "Inicia un comando de shell en segundo plano, devuelve el control y avisa cuando termine."),
            locale("hi-IN", summary = "शेल कमांड को बैकग्राउंड में शुरू करके तुरंत लौटें; पूरा होने पर सूचना दें।"),
            locale("ar-SA", summary = "شغّل أمر shell في الخلفية وعد فورًا كي يتابع الذكاء الاصطناعي عمله، ثم أرسل إشعارًا عند اكتماله."),
            locale("pt-BR", summary = "Inicie um comando shell em segundo plano, retorne logo para o trabalho continuar e avise quando terminar.")
        ),
        help = "Launch with closed stdin; return process_id only after actual startup and registration. Continue useful work immediately. Completion, failure, timeout or stop is delivered once as system_notification; do not poll merely to wait. Use shell.read for bounded diagnostic output, shell.list for state and shell.stop for cleanup. Omitted timeout_ms permits running until exit/stop; a supplied timeout is enforced. Declare reads/writes/network effects. Attach or replace a watch on this same process with shell.watch(process_id, policy); omit/null policy to remove it. A process-targeted watch never spawns a second command or a second background operation. Use shell.open when later interactive input is required."
    )]
    async fn invoke_spawn(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellLaunchInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Spawn {
                shell: args.shell,
                command: Box::new(args.command),
            },
            context,
        )
    }

    #[tool(
        tags(execute, shell, ToolTag::Custom("background".to_owned()), ToolTag::Custom("monitor".to_owned()), ToolTag::Custom("watch".to_owned()), mutate),
        summary = "Start a command with readiness/selected output notifications, or attach, replace or remove the watch on an existing background shell process.",
        translations(
            locale("zh-CN", summary = "启动命令并按就绪或输出规则通知，或管理现有后台 Shell 进程的监听。"),
            locale("zh-TW", summary = "啟動命令並依就緒或輸出規則通知，或管理既有背景 Shell 程序的監看。"),
            locale("ja-JP", summary = "準備完了や指定出力を通知するコマンドを起動するか、既存のバックグラウンドプロセスの監視を設定します。"),
            locale("ko-KR", summary = "준비 상태나 지정한 출력 알림을 받도록 명령을 시작하거나 기존 백그라운드 프로세스의 감시를 관리합니다."),
            locale("fr-FR", summary = "Lancer une commande avec notifications ciblées ou gérer la surveillance d’un processus shell déjà lancé."),
            locale("de-DE", summary = "Einen Befehl mit Bereitschafts- oder Ausgabenachrichten starten oder die Überwachung eines laufenden Shell-Prozesses ändern."),
            locale("es-ES", summary = "Inicia un comando con avisos de disponibilidad o salida, o gestiona la supervisión de un proceso shell existente."),
            locale("hi-IN", summary = "तैयारी या चुने हुए आउटपुट की सूचनाओं के साथ कमांड शुरू करें, या चल रही प्रक्रिया की निगरानी बदलें।"),
            locale("ar-SA", summary = "ابدأ أمرًا مع إشعارات الجاهزية أو المخرجات المحددة، أو أدر مراقبة عملية shell تعمل في الخلفية."),
            locale("pt-BR", summary = "Inicie um comando com avisos de prontidão ou saída selecionada, ou ajuste o monitoramento de um processo shell existente.")
        ),
        help = "Provide either command with shell/effects (new launch) or process_id (existing noninteractive process); never both. Launch binds the watch before the process can output. A process target replaces its single policy and keeps the original launch/completion operation. Omit or pass null policy on a process target to remove the watch without stopping the process; an empty policy disables ordinary notifications. Attach observes future output by default; since_seq optionally scans retained raw logs once. Identical policy updates are idempotent. ready_pattern notifies once and leaves the service running. include_pattern is opt-in; omission sends no ordinary output notifications. notifications defaults to once; on_change deduplicates unchanged matches and coalesces changed matches to the latest, using notification_interval_ms (default 30000, min 1000). Readiness/completion bypass that throttle. success_pattern/failure_pattern stop the process tree; failure wins. pattern_kind applies to all patterns (regex default, literal available); preserve whitespace. quiet_period_ms stops successfully after no raw stdout/stderr activity. timeout_ms belongs only to the launch and remains enforced after watch changes/removal. Notification filters do not filter raw content capture. Read with shell.read and stop with shell.stop; do not poll merely to wait. PTYs and WebSocket subscriptions cannot receive this watch."
    )]
    async fn invoke_watch(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellWatchInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Watch {
                input: Box::new(args),
            },
            context,
        )
    }

    #[tool(
        tags(execute, shell, ToolTag::Custom("terminal".to_owned()), interactive, ToolTag::Custom("tty".to_owned()), ToolTag::Custom("pty".to_owned()), mutate),
        summary = "Open a persistent interactive shell terminal/PTY for a CLI, REPL or full-screen program; continue with shell.read and shell.write.",
        translations(
            locale("zh-CN", summary = "打开持久交互式 Shell/PTY，运行 CLI、REPL 或全屏程序；之后用 shell.read 和 shell.write 交互。"),
            locale("zh-TW", summary = "開啟常駐互動式 Shell/PTY 以執行 CLI、REPL 或全螢幕程式；後續用 shell.read 和 shell.write 操作。"),
            locale("ja-JP", summary = "CLI、REPL、全画面アプリ向けに対話型シェル／PTYを開き、shell.read と shell.write で操作します。"),
            locale("ko-KR", summary = "CLI, REPL 또는 전체 화면 프로그램용 대화형 셸/PTY를 열고 shell.read와 shell.write로 계속 조작합니다."),
            locale("fr-FR", summary = "Ouvrir un shell/PTY interactif persistant pour un CLI, un REPL ou un programme plein écran, puis utiliser shell.read et shell.write."),
            locale("de-DE", summary = "Eine dauerhafte interaktive Shell/PTY für CLI, REPL oder Vollbildprogramme öffnen und mit shell.read sowie shell.write bedienen."),
            locale("es-ES", summary = "Abre un shell/PTY interactivo persistente para CLI, REPL o programas a pantalla completa; continúa con shell.read y shell.write."),
            locale("hi-IN", summary = "CLI, REPL या फ़ुल-स्क्रीन प्रोग्राम के लिए स्थायी इंटरैक्टिव शेल/PTY खोलें; आगे shell.read और shell.write से काम करें।"),
            locale("ar-SA", summary = "افتح صدفة تفاعلية دائمة أو PTY لتشغيل CLI أو REPL أو برنامج ملء الشاشة، ثم تابع باستخدام shell.read وshell.write."),
            locale("pt-BR", summary = "Abra um shell/PTY interativo persistente para CLI, REPL ou programa de tela cheia e continue com shell.read e shell.write.")
        ),
        help = "Return process_id, output, last_seq, next_event_offset and state after confirmed startup. yield_time_ms (default 1000, max 30000) limits only the initial wait, never lifetime; timeout_ms is an optional lifetime deadline. Interactive exit does not send background completion notifications or wake the AI. max_output_bytes (1024–16384, default 16384) controls preview, not capture. With include_screen the available content budget is split between raw output and a bounded screen. Silence/prompt/yield is not exit. Continue with shell.read or exact-input shell.write; shell.signal interrupts/terminates/kills, shell.resize changes dimensions and shell.stop cleans up. Declare initial effects and subsequent write effects."
    )]
    async fn invoke_open(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellOpenInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Open {
                input: Box::new(args),
            },
            context,
        )
    }

    #[tool(
        tags(query, discovery, shell, read_only),
        summary = "List this session's background shell jobs, watched commands and interactive terminals, including their type and state.",
        translations(
            locale(
                "zh-CN",
                summary = "列出当前会话的后台 Shell 任务、监听命令和交互终端及其类型与状态。"
            ),
            locale(
                "zh-TW",
                summary = "列出目前工作階段的背景 Shell 工作、監看命令與互動式終端及其類型和狀態。"
            ),
            locale(
                "ja-JP",
                summary = "このセッションのバックグラウンドジョブ、監視中のコマンド、対話型ターミナルと各状態を一覧表示します。"
            ),
            locale(
                "ko-KR",
                summary = "현재 세션의 백그라운드 셸 작업, 감시 명령 및 대화형 터미널과 상태를 나열합니다."
            ),
            locale(
                "fr-FR",
                summary = "Lister les tâches shell en arrière-plan, commandes surveillées et terminaux interactifs de cette session avec leur état."
            ),
            locale(
                "de-DE",
                summary = "Hintergrundjobs, überwachte Befehle und interaktive Terminals dieser Sitzung samt Status auflisten."
            ),
            locale(
                "es-ES",
                summary = "Muestra los procesos de shell en segundo plano, comandos supervisados y terminales interactivos de esta sesión con su estado."
            ),
            locale(
                "hi-IN",
                summary = "इस सत्र के बैकग्राउंड शेल कार्य, निगरानी वाले कमांड और इंटरैक्टिव टर्मिनल उनकी स्थिति सहित दिखाएँ।"
            ),
            locale(
                "ar-SA",
                summary = "اعرض مهام shell الخلفية والأوامر الخاضعة للمراقبة والطرفيات التفاعلية في هذه الجلسة مع حالتها."
            ),
            locale(
                "pt-BR",
                summary = "Liste os trabalhos shell em segundo plano, comandos monitorados e terminais interativos desta sessão com seus estados."
            )
        )
    )]
    async fn invoke_list(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        invoke(ShellToolInput::List {}, context)
    }

    #[tool(
        tags(query, shell, ToolTag::Custom("background".to_owned()), read_only),
        summary = "Read bounded output with an explicit replay cursor; shell.read is the unified, automatically consuming entry point.",
        translations(
            locale("zh-CN", summary = "通过明确的回放游标读取有限输出；通常用 shell.read 自动读取新内容。"),
            locale("zh-TW", summary = "透過明確的回放游標讀取有限輸出；一般使用 shell.read 自動接收新內容。"),
            locale("ja-JP", summary = "カーソルを指定して出力を再生します。新着分の読み取りには自動で消費する shell.read を使います。"),
            locale("ko-KR", summary = "커서를 지정해 제한된 출력을 다시 읽습니다. 새 출력은 자동으로 소비하는 shell.read를 사용하세요."),
            locale("fr-FR", summary = "Relire une sortie limitée avec un curseur explicite ; shell.read est le point d’entrée pour les nouvelles données."),
            locale("de-DE", summary = "Begrenzte Ausgabe mit explizitem Cursor erneut lesen; shell.read liest neue Ausgabe automatisch weiter."),
            locale("es-ES", summary = "Relee una salida limitada con un cursor explícito; shell.read es la opción habitual para consumir lo nuevo."),
            locale("hi-IN", summary = "स्पष्ट कर्सर से सीमित आउटपुट दोबारा पढ़ें; नई सामग्री के लिए shell.read अपने-आप आगे बढ़ता है।"),
            locale("ar-SA", summary = "أعد قراءة مخرجات محدودة بمؤشر صريح؛ استخدم shell.read عادةً لقراءة الجديد تلقائيًا."),
            locale("pt-BR", summary = "Releia uma saída limitada com cursor explícito; shell.read é a opção usual para consumir o que chegou.")
        ),
        help = "Explicit replay of shell process output. since_seq defaults to 0; continue with last_seq AND next_event_offset as event_offset whenever nonzero. last_seq refers to fully consumed events; partially returned event text is resumable without loss. max_output_bytes (1024–16384, default 16384) includes escaping and event structure, with room reserved for lifecycle/cursor metadata. limit caps event count; wait_ms is a diagnostic wait (max 30000). has_more describes buffered output. output_resource identifies the canonical retained content; capture completeness is separate from the preview. Use shell.read without since_seq for new unread output. Background jobs notify when they finish; do not poll to wait."
    )]
    async fn invoke_logs(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessLogsInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Logs {
                process_id: args.process_id,
                since_seq: args.since_seq,
                event_offset: args.event_offset,
                max_output_bytes: args.max_output_bytes,
                limit: args.limit,
                wait_ms: args.wait_ms,
            },
            context,
        )
    }

    #[tool(
        tags(query, shell, ToolTag::Custom("terminal".to_owned()), interactive, ToolTag::Custom("tty".to_owned()), ToolTag::Custom("pty".to_owned()), read_only),
        summary = "Read bounded incremental output and state from any owned background shell job or interactive terminal, without input.",
        translations(
            locale("zh-CN", summary = "增量读取自有后台 Shell 任务或交互终端的状态和有限输出，不发送输入。"),
            locale("zh-TW", summary = "增量讀取自有背景 Shell 工作或互動式終端的狀態與有限輸出，不傳送輸入。"),
            locale("ja-JP", summary = "所有するバックグラウンドジョブや対話型ターミナルから入力を送らずに、状態と出力を少しずつ読み取ります。"),
            locale("ko-KR", summary = "소유한 백그라운드 셸 작업이나 대화형 터미널의 상태와 출력을 입력 없이 이어서 읽습니다."),
            locale("fr-FR", summary = "Lire progressivement l’état et la sortie limitée d’un shell détenu, sans lui envoyer d’entrée."),
            locale("de-DE", summary = "Status und begrenzte Ausgabe eines eigenen Shell-Jobs oder Terminals inkrementell lesen, ohne Eingaben zu senden."),
            locale("es-ES", summary = "Lee de forma incremental el estado y la salida limitada de un proceso propio, sin enviarle entrada."),
            locale("hi-IN", summary = "अपने बैकग्राउंड शेल कार्य या इंटरैक्टिव टर्मिनल की स्थिति और आउटपुट पढ़ें, बिना इनपुट भेजे।"),
            locale("ar-SA", summary = "اقرأ تدريجيًا حالة ومخرجات مهمة shell الخلفية أو الطرفية التفاعلية المملوكة لك، من دون إرسال إدخال."),
            locale("pt-BR", summary = "Leia aos poucos o estado e a saída limitada de um processo seu, sem enviar entrada.")
        ),
        help = "Works with process_id from shell.spawn, shell.watch or shell.open. Omit since_seq and event_offset to consume unread output; explicit since_seq replays without changing the automatic cursor. A partial event returns last_seq for fully consumed events and next_event_offset for the next event; pass both as since_seq/event_offset to resume. A nonzero event_offset requires since_seq and must preserve UTF-8 boundaries. Reads/writes on one process serialize automatic consumption; cancellation leaves it alive. wait_ms defaults to 250 (max 30000) and is a bounded output wait, not completion. max_output_bytes is 1024–16384 (default 16384), counting escaping/event structure and reserving metadata. include_screen is for PTYs only; enabling it splits the content budget with a bounded screen. ready is independent of terminal status. has_more describes the rolling buffer. output_resource identifies retained content outside the rolling preview. Background completion is notified once, so do not poll merely to wait."
    )]
    async fn invoke_read(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellReadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(ShellToolInput::Read { input: args }, context)
    }

    #[tool(
        tags(mutate, execute, shell, ToolTag::Custom("terminal".to_owned()), interactive, ToolTag::Custom("tty".to_owned()), ToolTag::Custom("pty".to_owned())),
        summary = "Send exact nonempty input to an interactive shell.open terminal and collect a bounded, resumable response.",
        translations(
            locale("zh-CN", summary = "向 shell.open 交互终端发送原样输入，并读取可续读的有限响应。"),
            locale("zh-TW", summary = "向 shell.open 互動式終端傳送原樣輸入，並讀取可續讀的有限回應。"),
            locale("ja-JP", summary = "shell.open の対話型ターミナルに入力をそのまま送り、続きを取得できる範囲内の応答を読み取ります。"),
            locale("ko-KR", summary = "shell.open 대화형 터미널에 입력을 그대로 보내고 이어서 읽을 수 있는 제한된 응답을 받습니다."),
            locale("fr-FR", summary = "Envoyer une entrée exacte au terminal interactif shell.open et lire une réponse limitée, reprenable par curseur."),
            locale("de-DE", summary = "Eingaben unverändert an das interaktive shell.open-Terminal senden und eine fortsetzbare, begrenzte Antwort lesen."),
            locale("es-ES", summary = "Envía la entrada exacta al terminal interactivo de shell.open y obtiene una respuesta limitada que puede continuarse."),
            locale("hi-IN", summary = "shell.open इंटरैक्टिव टर्मिनल को इनपुट जस का तस भेजें और आगे पढ़े जा सकने वाला सीमित उत्तर लें।"),
            locale("ar-SA", summary = "أرسل الإدخال كما هو إلى طرفية shell.open التفاعلية واقرأ استجابة محدودة قابلة للاستكمال."),
            locale("pt-BR", summary = "Envie a entrada exata ao terminal interativo de shell.open e leia uma resposta limitada que pode ser retomada.")
        ),
        help = "chars is exact terminal input: never trim or append a newline. Send \\r for Enter, \\u0003 for Ctrl-C, \\u0004 for Ctrl-D, \\t for Tab, or terminal escape sequences. Use shell.read without input. Omit since_seq/event_offset for unread output; explicit cursors replay without changing automatic consumption. Continue a partial result with last_seq and next_event_offset as since_seq/event_offset. wait_ms defaults to 250 (max 30000), independent of lifetime. max_output_bytes is 1024–16384 (default 16384); include_screen splits that content budget. Cursor/effect validation occurs before sending input. Declare reads/writes/network effects of the entered operation. Requires the owning session/workspace. A partial-write error requests termination; never blindly resend the full input. Use shell.signal for out-of-band interruption and shell.stop for cleanup."
    )]
    async fn invoke_write(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellWriteInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(ShellToolInput::Write { input: args }, context)
    }

    #[tool(
        tags(mutate, execute, shell),
        summary = "Stop an owned background shell job, watched command or interactive terminal and clean up its process tree.",
        translations(
            locale(
                "zh-CN",
                summary = "停止自有后台 Shell 任务、监听命令或交互终端，并清理其进程树。"
            ),
            locale(
                "zh-TW",
                summary = "停止自有背景 Shell 工作、監看命令或互動式終端，並清理其程序樹。"
            ),
            locale(
                "ja-JP",
                summary = "所有するバックグラウンドジョブや対話型ターミナルを停止し、プロセスツリーを終了します。"
            ),
            locale(
                "ko-KR",
                summary = "소유한 백그라운드 셸 작업, 감시 명령 또는 대화형 터미널을 중지하고 프로세스 트리를 정리합니다."
            ),
            locale(
                "fr-FR",
                summary = "Arrêter un shell détenu et nettoyer son arbre de processus."
            ),
            locale(
                "de-DE",
                summary = "Einen eigenen Shell-Job oder ein Terminal stoppen und den Prozessbaum bereinigen."
            ),
            locale(
                "es-ES",
                summary = "Detiene un proceso shell propio y limpia su árbol de procesos."
            ),
            locale(
                "hi-IN",
                summary = "अपने बैकग्राउंड शेल कार्य या टर्मिनल को रोककर उसकी प्रक्रिया-श्रृंखला साफ़ करें।"
            ),
            locale(
                "ar-SA",
                summary = "أوقف مهمة shell أو طرفية تفاعلية تملكها ونظّف شجرة العمليات التابعة لها."
            ),
            locale(
                "pt-BR",
                summary = "Pare um processo shell ou terminal seu e encerre a árvore de processos correspondente."
            )
        )
    )]
    async fn invoke_stop(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessStopInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Stop {
                process_id: args.process_id,
            },
            context,
        )
    }

    #[tool(
        tags(mutate, shell, ToolTag::Custom("terminal".to_owned()), ToolTag::Custom("tty".to_owned()), ToolTag::Custom("pty".to_owned())),
        summary = "Resize an interactive shell.open terminal in character rows and columns.",
        translations(
            locale("zh-CN", summary = "调整 shell.open 交互终端的字符行数和列数。"),
            locale("zh-TW", summary = "調整 shell.open 互動式終端的字元列數與行數。"),
            locale("ja-JP", summary = "shell.open の対話型ターミナルの行数と列数を変更します。"),
            locale("ko-KR", summary = "shell.open 대화형 터미널의 문자 행과 열 크기를 바꿉니다."),
            locale("fr-FR", summary = "Redimensionner le terminal shell.open en lignes et colonnes de caractères."),
            locale("de-DE", summary = "Die Größe des shell.open-Terminals in Zeichenzeilen und -spalten ändern."),
            locale("es-ES", summary = "Cambia el tamaño del terminal de shell.open en filas y columnas de caracteres."),
            locale("hi-IN", summary = "shell.open टर्मिनल की पंक्तियों और स्तंभों का आकार बदलें।"),
            locale("ar-SA", summary = "غيّر حجم طرفية shell.open بعدد صفوف وأعمدة الأحرف."),
            locale("pt-BR", summary = "Redimensione o terminal de shell.open em linhas e colunas de caracteres.")
        )
    )]
    async fn invoke_resize(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessResizeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Resize {
                process_id: args.process_id,
                rows: args.rows,
                cols: args.cols,
            },
            context,
        )
    }

    #[tool(
        tags(mutate, execute, shell, ToolTag::Custom("terminal".to_owned()), interactive, ToolTag::Custom("tty".to_owned()), ToolTag::Custom("pty".to_owned())),
        summary = "Interrupt, gracefully terminate or kill an owned interactive shell terminal.",
        translations(
            locale("zh-CN", summary = "中断、优雅终止或强制结束自有交互式 Shell 终端。"),
            locale("zh-TW", summary = "中斷、正常終止或強制結束自有互動式 Shell 終端。"),
            locale("ja-JP", summary = "所有する対話型シェルターミナルを割り込み、正常終了、または強制終了します。"),
            locale("ko-KR", summary = "소유한 대화형 셸 터미널을 인터럽트하거나 정상 또는 강제 종료합니다."),
            locale("fr-FR", summary = "Interrompre ou terminer proprement ou de force un terminal shell interactif détenu."),
            locale("de-DE", summary = "Ein eigenes interaktives Shell-Terminal unterbrechen oder geordnet beziehungsweise sofort beenden."),
            locale("es-ES", summary = "Interrumpe o finaliza de forma ordenada o forzada un terminal shell interactivo propio."),
            locale("hi-IN", summary = "अपने इंटरैक्टिव शेल टर्मिनल को बाधित करें या सामान्य अथवा बलपूर्वक बंद करें।"),
            locale("ar-SA", summary = "قاطع طرفية shell تفاعلية تملكها أو أنهِها بشكل سلس أو قسري."),
            locale("pt-BR", summary = "Interrompa ou encerre de forma normal ou forçada um terminal shell interativo seu.")
        ),
        help = "interrupt targets the Unix foreground process group without closing the shell; ConPTY uses terminal Ctrl-C. terminate requests graceful cleanup then kills remaining jobs; kill skips the grace period. Out-of-band interruption also works for raw-mode programs. Requires the owning session/workspace. shell.stop is equivalent to terminate."
    )]
    async fn invoke_signal(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessSignalInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Signal {
                process_id: args.process_id,
                signal: args.signal,
            },
            context,
        )
    }
}

fn invoke(input: ShellToolInput, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
    router::invoke_tool(
        "shell",
        json_input(input)?,
        context.session_id,
        context.call_id,
    )
}

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params_error(&err))
}

#[cfg(test)]
mod tests {
    use super::ShellPlugin;
    use agena_plugin_host::sdk::Plugin;

    #[test]
    fn manifest_exposes_shell_tools_under_the_shell_plugin() {
        let manifest = ShellPlugin.manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "shell");
        assert_eq!(
            tool_names,
            [
                "exec", "spawn", "watch", "open", "list", "logs", "read", "write", "stop",
                "resize", "signal"
            ]
        );
        for name in ["exec", "spawn", "watch", "open"] {
            let tool = manifest
                .tools
                .iter()
                .find(|tool| tool.name == name)
                .expect("shell launch manifest");
            let schema = serde_json::to_string(&tool.input_schema()).expect("serialize schema");
            assert!(!schema.contains("run_in_background"));
            assert!(!schema.contains("\"tty\""));
            let examples =
                agena_runtime_tools::tool::definition::schema_example_texts(&tool.input_schema());
            let example_text = examples.first().expect("shell generated example");
            let example_text =
                &example_text[example_text.find('{').expect("object shell example")..];
            let example: serde_json::Value =
                serde_json::from_str(example_text).expect("shell example must be JSON");
            assert!(example.get("reads").is_some());
            assert!(example.get("writes").is_some());
            assert!(example.get("network").is_some());
        }
    }
}
