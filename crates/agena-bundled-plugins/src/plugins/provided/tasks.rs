mod admission;
#[cfg(test)]
mod fault_tests;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::part::TaskToolInput;
use crate::plugins::provided::workflow::{WorkflowPlugin, WorkflowPluginConfig};
use agena_macros::ToolInput;
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::host_api::{
    CancelSubtaskRequest, HostCallbackContext, HostStorageGetRequest, HostStorageListRequest,
    HostStorageScope, HostStorageSetRequest, MessageSubtaskRequest, ReadSubtaskOutputRequest,
    RunSubtaskModelSelection, RunSubtaskRequest, RunSubtaskResponse, RunSubtaskStatus,
    current_host_callback_context, run_in_host_callback_context,
};
use agena_plugin_host::sdk::{
    InitContext, InitOutcome, Result as SdkResult, SessionEndInput, ToolInvokeContext,
    ToolInvokeOutput,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

pub(crate) const TASKS_PLUGIN_ID: &str = "agena.tasks";
const TASK_STORAGE_NAMESPACE: &str = "async_tasks";
/// Prevent one parent session from filling the runtime with unbounded child
/// executions. This is deliberately a per-parent admission boundary; global
/// provider capacity remains owned by the runtime/provider layer.
const MAX_ACTIVE_TASKS_PER_PARENT: usize = 8;

pub(crate) struct TasksPlugin {
    inner: WorkflowPlugin,
    tasks: Arc<Mutex<BTreeMap<String, Arc<AsyncTaskEntry>>>>,
    admission: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Debug)]
struct AsyncTaskEntry {
    state: Mutex<AsyncTaskState>,
    notify: Arc<Notify>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AsyncTaskState {
    #[serde(default)]
    run_epoch: u64,
    #[serde(default)]
    storage_warning: Option<String>,
    task_id: String,
    parent_session_id: i64,
    description: String,
    /// Original instruction is retained only in plugin-private durable storage,
    /// keyed by the parent session, so a user can make an explicit post-restart
    /// recovery decision. It is never
    /// replayed automatically, because the child session may already contain
    /// that user message when a process died before acknowledging completion.
    prompt: String,
    status: String,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
    response: Option<RunSubtaskResponse>,
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection: Option<RunSubtaskModelSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_cost_microusd: Option<u64>,
    #[serde(default)]
    budget_exceeded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput, Default)]
#[serde(deny_unknown_fields)]
struct TaskListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("task_id"), non_empty("task_id"))]
#[serde(deny_unknown_fields)]
struct TaskIdInput {
    task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("task_id", "message"), non_empty("task_id", "message"))]
#[serde(deny_unknown_fields)]
struct TaskMessageInput {
    task_id: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("task_id", "prompt"),
    non_empty("task_id", "prompt"),
    minimum("timeout_ms", 1),
    minimum("max_tokens", 1),
    minimum("max_cost_microusd", 1)
)]
#[serde(deny_unknown_fields)]
struct TaskFollowupInput {
    task_id: String,
    prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_cost_microusd: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("task_id"),
    non_empty("task_id"),
    minimum("cursor", 0),
    minimum("limit", 1),
    maximum("limit", 500)
)]
#[serde(deny_unknown_fields)]
struct TaskOutputInput {
    task_id: String,
    #[serde(default)]
    cursor: i64,
    #[serde(default = "default_output_limit")]
    limit: u32,
}

const fn default_output_limit() -> u32 {
    100
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "tasks",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Delegated subtask orchestration tools.",
    translations(
        locale("zh-CN", summary = "委派子任务编排工具。"),
        locale("zh-TW", summary = "委派子工作編排工具。"),
        locale("ja-JP", summary = "委任したサブタスクを管理するツールです。"),
        locale("ko-KR", summary = "하위 작업 위임과 관리를 위한 도구입니다."),
        locale("fr-FR", summary = "Outils de délégation et de gestion de sous-tâches."),
        locale("de-DE", summary = "Werkzeuge zum Delegieren und Verwalten von Teilaufgaben."),
        locale("es-ES", summary = "Herramientas para delegar y coordinar subtareas."),
        locale("hi-IN", summary = "उप-कार्य सौंपने और उन्हें व्यवस्थित करने के टूल।"),
        locale("ar-SA", summary = "أدوات لتفويض المهام الفرعية وإدارتها."),
        locale("pt-BR", summary = "Ferramentas para delegar e coordenar subtarefas.")
    ),
)]
impl TasksPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
            tasks: Arc::new(Mutex::new(BTreeMap::new())),
            admission: Arc::new(tokio::sync::Mutex::new(())),
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
        tags(subtask, execute, task),
        summary = "Delegate a bounded task to a subagent session. Set `run_in_background` to run it in the background and be notified when it settles. Attach command names in `commands` so the child session applies their instructions as task guidance.",
        translations(
            locale(
                "zh-CN",
                summary = "将有明确边界的任务委派给子代理会话。设置 `run_in_background` 可让任务在后台执行，并在结束时通知你。通过 `commands` 附加命令名称，让子会话将对应说明作为任务指导。",
                help = "当工作适合某个现有命令或子代理类型、需要并行处理相互独立的事项，或回答问题必须跨多个文件阅读时，使用此工具委派任务，并由你整理结论，而不是转发文件内容。若只需查询一个已知文件、符号或值，请直接搜索；委派搜索后不要再亲自重复搜索，等待子任务结果。小任务自行完成，不要把一个任务拆成许多子任务；能够自行验证时就直接验证，不要重复已委派的工作。不要委派理解工作：给子代理明确的文件路径、行号和修改要求，再检查它的结果。`commands` 可填写命令名、斜杠命令或别名，例如给审查任务指定只读审查命令，或给探索任务指定 explore 命令；子会话会收到这些命令所对应的说明并遵循。未知命令会在子任务启动前被拒绝。使用 `agena.commands` 插件的 `list` 工具查看当前工作区提供的命令。默认情况下，任务会在当前调用中执行完毕并返回最终结果。设置 `run_in_background: true` 后会立即返回任务 id，完成后通过 `system_notification` 通知；不要轮询 `tasks.get` 或 `tasks.output` 等待完成。"
            ),
            locale(
                "zh-TW",
                summary = "將範圍明確的工作委派給子代理工作階段。將 `run_in_background` 設為 true 可在背景執行，並於完成時通知你。透過 `commands` 附上命令名稱，讓子工作階段依照命令說明執行。"
            ),
            locale(
                "ja-JP",
                summary = "範囲を絞ったタスクをサブエージェントのセッションに委任します。`run_in_background` を true にするとバックグラウンドで実行し、終了時に通知します。`commands` にコマンド名を指定すると、その説明を子セッションの指針として引き継ぎます。"
            ),
            locale(
                "ko-KR",
                summary = "범위가 명확한 작업을 하위 에이전트 세션에 맡깁니다. `run_in_background`를 true로 설정하면 백그라운드에서 실행하고 완료 시 알립니다. `commands`에 명령 이름을 지정하면 해당 설명이 하위 세션의 작업 지침으로 전달됩니다."
            ),
            locale(
                "fr-FR",
                summary = "Déléguer une tâche bien délimitée à une session de sous-agent. Avec `run_in_background`, elle s’exécute en arrière-plan et une notification est envoyée à la fin. Indiquez des noms de commandes dans `commands` pour transmettre leurs consignes à la session enfant."
            ),
            locale(
                "de-DE",
                summary = "Eine klar abgegrenzte Aufgabe an eine Subagent-Sitzung delegieren. Mit `run_in_background` läuft sie im Hintergrund und meldet sich nach Abschluss. Über `commands` lassen sich Befehlsnamen mitgeben, deren Anweisungen die Sitzung übernehmen soll."
            ),
            locale(
                "es-ES",
                summary = "Delega una tarea acotada a una sesión de subagente. Con `run_in_background` se ejecuta en segundo plano y recibirás un aviso al terminar. Indica nombres de comandos en `commands` para que la sesión hija siga sus instrucciones."
            ),
            locale(
                "hi-IN",
                summary = "सीमित दायरे वाला काम उप-एजेंट सत्र को सौंपें। `run_in_background` चालू करने पर काम बैकग्राउंड में चलेगा और पूरा होने पर सूचना मिलेगी। `commands` में कमांड नाम दें ताकि उनका मार्गदर्शन चाइल्ड सत्र तक पहुँचे।"
            ),
            locale(
                "ar-SA",
                summary = "فوّض مهمة محددة النطاق إلى جلسة وكيل فرعي. عند تفعيل `run_in_background` تعمل المهمة في الخلفية ويصلك إشعار عند انتهائها. أدرج أسماء الأوامر في `commands` لنقل تعليماتها إلى الجلسة التابعة."
            ),
            locale(
                "pt-BR",
                summary = "Delegue uma tarefa bem delimitada a uma sessão de subagente. Com `run_in_background`, ela roda em segundo plano e você recebe uma notificação ao terminar. Informe nomes de comandos em `commands` para que as instruções sejam repassadas à sessão filha."
            )
        ),
        help = "Reach for this tool when the work matches an available command or subagent type, when you have independent work to run in parallel, or when answering would mean reading across several files — delegate it and you keep the conclusion, not the file dumps. For a single-fact lookup where you already know the file, symbol, or value, search directly; once you have delegated a search, do not also run it yourself — wait for the result. Do small tasks yourself instead of delegating; do not fan out a single task into many subtasks; verify inline instead of delegating when you can; do not redo work you already delegated. Never delegate understanding: brief the subagent with concrete file paths, line numbers, and what to change, then check its result. Set `commands` to command names, slash spellings or aliases (for example a read-only review command for a review task, or an explore command for an exploration task); the child session receives the instructions those commands name and should follow them. Unknown names are rejected before the subtask starts. Use the `agena.commands` plugin's `list` tool to discover what this workspace offers. By default the subtask runs inline and this call returns its final result before returning. With `run_in_background: true` the subtask runs in the background: the tool returns immediately with a task id and the result is delivered as a `system_notification` when it settles — do not poll tasks.get/tasks.output waiting for it."
    )]
    async fn run(
        &self,
        input: &TaskToolInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        if !input.run_in_background {
            return self.inner.invoke_task(input).await;
        }
        self.hydrate_session_tasks(context).await?;
        let _admission = self.admission.lock().await;
        let task_id = input
            .task_id
            .clone()
            .unwrap_or_else(|| format!("task_{}", uuid::Uuid::new_v4().simple()));
        let selection = input
            .selection
            .as_ref()
            .map(|selection| RunSubtaskModelSelection {
                provider: selection.provider.clone(),
                adapter: selection.adapter.clone(),
                model: selection.model.clone(),
                thinking_mode: selection.thinking_mode.clone(),
                speed_mode: selection.speed_mode.clone(),
                verbosity: selection.verbosity.clone(),
                parallel_tool_calls: selection.parallel_tool_calls,
            });
        let state = AsyncTaskState {
            run_epoch: 1,
            storage_warning: None,
            task_id: task_id.clone(),
            parent_session_id: context.session_id,
            description: input.description.clone(),
            prompt: input.prompt.clone(),
            status: "running".to_string(),
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            finished_at_ms: None,
            response: None,
            error: None,
            selection: selection.clone(),
            timeout_ms: input.timeout_ms,
            max_tokens: input.max_tokens,
            max_cost_microusd: input.max_cost_microusd,
            budget_exceeded: false,
        };
        let reservation = self.reserve_task(state.clone(), false)?;
        let host = self.inner.host()?;
        let request = RunSubtaskRequest {
            parent_session_id: Some(context.session_id),
            run_in_background: true,
            launch_call_id: Some(context.call_id),
            description: input.description.clone(),
            prompt: input.prompt.clone(),
            commands: input.commands.clone(),
            task_id: Some(task_id.clone()),
            selection,
            timeout_ms: input.timeout_ms,
            max_tokens: input.max_tokens,
            max_cost_microusd: input.max_cost_microusd,
        };
        persist_task_state(&host, callback_context(context), &state).await?;
        let entry = reservation.commit();
        spawn_task(
            host,
            Arc::clone(&entry),
            request,
            Arc::clone(&self.admission),
            Arc::clone(&self.tasks),
        );
        Ok(task_output(
            "Start task",
            format!(
                "Started task '{task_id}' in the background. You will be notified when it completes — do not poll; continue with other work in the meantime."
            ),
            vec![state],
            false,
        ))
    }

    #[tool(
        tags(subtask, query, discovery, read_only, task),
        summary = "List delegated background tasks.",
        translations(
            locale("zh-CN", summary = "列出已委派的后台任务。"),
            locale("zh-TW", summary = "列出已委派的背景工作。"),
            locale(
                "ja-JP",
                summary = "委任済みのバックグラウンドタスクを一覧表示します。"
            ),
            locale("ko-KR", summary = "위임된 백그라운드 작업을 나열합니다."),
            locale("fr-FR", summary = "Lister les tâches déléguées en arrière-plan."),
            locale("de-DE", summary = "Delegierte Hintergrundaufgaben auflisten."),
            locale("es-ES", summary = "Muestra las tareas delegadas en segundo plano."),
            locale("hi-IN", summary = "सौंपे गए बैकग्राउंड कार्यों की सूची दिखाएँ।"),
            locale("ar-SA", summary = "اعرض المهام المفوّضة التي تعمل في الخلفية."),
            locale("pt-BR", summary = "Liste as tarefas delegadas em segundo plano.")
        )
    )]
    async fn list(
        &self,
        input: &TaskListInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let states = self
            .tasks
            .lock()
            .map_err(|_| agena_plugin_host::PluginError::internal("tasks registry lock poisoned"))?
            .values()
            .map(|entry| recover_task_state(entry).clone())
            .filter(|state| state.parent_session_id == context.session_id)
            .filter(|state| {
                input
                    .status
                    .as_ref()
                    .is_none_or(|status| state.status.eq_ignore_ascii_case(status.trim()))
            })
            .collect::<Vec<_>>();
        Ok(task_output(
            "List tasks",
            format!("{} delegated task(s).", states.len()),
            states,
            false,
        ))
    }

    #[tool(
        tags(subtask, query, read_only, task),
        summary = "Get delegated task metadata and terminal result.",
        translations(
            locale("zh-CN", summary = "查看已委派任务的状态信息和最终结果。"),
            locale("zh-TW", summary = "查看已委派工作的中繼資料與最終結果。"),
            locale(
                "ja-JP",
                summary = "委任したタスクのメタデータと最終結果を取得します。"
            ),
            locale(
                "ko-KR",
                summary = "위임된 작업의 메타데이터와 최종 결과를 가져옵니다."
            ),
            locale(
                "fr-FR",
                summary = "Consulter les métadonnées et le résultat final d’une tâche déléguée."
            ),
            locale(
                "de-DE",
                summary = "Metadaten und Endergebnis einer delegierten Aufgabe abrufen."
            ),
            locale(
                "es-ES",
                summary = "Consulta los metadatos y el resultado final de una tarea delegada."
            ),
            locale("hi-IN", summary = "सौंपे गए कार्य का मेटाडेटा और अंतिम परिणाम देखें।"),
            locale(
                "ar-SA",
                summary = "اعرض بيانات المهمة المفوّضة الوصفية ونتيجتها النهائية."
            ),
            locale(
                "pt-BR",
                summary = "Consulte os metadados e o resultado final de uma tarefa delegada."
            )
        )
    )]
    async fn get(
        &self,
        input: &TaskIdInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let state =
            entry_state_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        Ok(task_output(
            "Task details",
            format!("Task '{}' is {}.", input.task_id, state.status),
            vec![state],
            false,
        ))
    }

    #[tool(
        tags(subtask, query, read_only, task),
        summary = "Read incremental delegated-task transcript output after a cursor.",
        translations(
            locale("zh-CN", summary = "从指定游标开始读取已委派任务新增的会话输出。"),
            locale("zh-TW", summary = "從指定游標開始讀取委派工作新增的對話輸出。"),
            locale(
                "ja-JP",
                summary = "指定したカーソル以降の、委任タスクの新しい会話出力を読み取ります。"
            ),
            locale(
                "ko-KR",
                summary = "지정한 커서 이후에 추가된 위임 작업의 대화 출력을 읽습니다."
            ),
            locale(
                "fr-FR",
                summary = "Lire les nouvelles sorties de conversation d’une tâche déléguée à partir d’un curseur."
            ),
            locale(
                "de-DE",
                summary = "Neue Gesprächsausgaben einer delegierten Aufgabe ab einem Cursor lesen."
            ),
            locale(
                "es-ES",
                summary = "Lee la nueva salida de conversación de una tarea delegada desde un cursor."
            ),
            locale("hi-IN", summary = "कर्सर के बाद से सौंपे गए कार्य का नया संवाद आउटपुट पढ़ें।"),
            locale(
                "ar-SA",
                summary = "اقرأ مخرجات المحادثة الجديدة للمهمة المفوّضة بدءًا من مؤشر محدد."
            ),
            locale(
                "pt-BR",
                summary = "Leia as novas saídas de conversa de uma tarefa delegada a partir de um cursor."
            )
        )
    )]
    async fn output(
        &self,
        input: &TaskOutputInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let state =
            entry_state_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        let output = self
            .inner
            .host()?
            .read_subtask_output(ReadSubtaskOutputRequest {
                parent_session_id: Some(state.parent_session_id),
                task_id: input.task_id.clone(),
                after_cursor: input.cursor,
                limit: input.limit,
            })
            .await?;
        let text = if output.chunks.is_empty() {
            state.error.clone().unwrap_or_else(|| {
                format!(
                    "No new task output after cursor {} (status: {}).",
                    input.cursor, state.status
                )
            })
        } else {
            output
                .chunks
                .iter()
                .map(|chunk| format!("[{}] {}", chunk.role, chunk.text))
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        Ok(ToolInvokeOutput::from_parts(
            "Task output",
            if output.has_more {
                format!(
                    "{} chunks · {} · more available",
                    output.chunks.len(),
                    state.status
                )
            } else {
                format!("{} chunks · {}", output.chunks.len(), state.status)
            },
            text,
            Some(serde_json::json!({
                "task": state,
                "chunks": output.chunks,
                "next_cursor": output.next_cursor,
                "has_more": output.has_more,
            })),
            BTreeMap::from([
                ("next_cursor".to_string(), output.next_cursor.to_string()),
                ("has_more".to_string(), output.has_more.to_string()),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        tags(subtask, mutate, task),
        summary = "Cancel a running delegated task and its child execution.",
        translations(
            locale("zh-CN", summary = "取消正在运行的已委派任务及其子会话执行。"),
            locale("zh-TW", summary = "取消執行中的委派工作及其子工作階段。"),
            locale(
                "ja-JP",
                summary = "実行中の委任タスクと子セッションの処理をキャンセルします。"
            ),
            locale(
                "ko-KR",
                summary = "실행 중인 위임 작업과 하위 세션 실행을 취소합니다."
            ),
            locale(
                "fr-FR",
                summary = "Annuler une tâche déléguée en cours et son exécution dans la session enfant."
            ),
            locale(
                "de-DE",
                summary = "Eine laufende delegierte Aufgabe und ihre Ausführung in der Kind-Sitzung abbrechen."
            ),
            locale(
                "es-ES",
                summary = "Cancela una tarea delegada en curso y su ejecución en la sesión hija."
            ),
            locale("hi-IN", summary = "चल रहे सौंपे गए कार्य और चाइल्ड सत्र की प्रक्रिया रद्द करें।"),
            locale(
                "ar-SA",
                summary = "ألغِ المهمة المفوّضة قيد التشغيل وتنفيذها في الجلسة التابعة."
            ),
            locale(
                "pt-BR",
                summary = "Cancele uma tarefa delegada em andamento e a execução na sessão filha."
            )
        )
    )]
    async fn cancel(
        &self,
        input: &TaskIdInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let _admission = self.admission.lock().await;
        let entry = task_entry_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        let parent_session_id = lock_state(&entry)?.parent_session_id;
        let response = self
            .inner
            .host()?
            .cancel_subtask(CancelSubtaskRequest {
                parent_session_id: Some(parent_session_id),
                task_id: input.task_id.clone(),
            })
            .await?;
        {
            let mut state = lock_state(&entry)?;
            if response.accepted && !is_terminal(&state.status) {
                state.status = "cancelling".to_string();
            }
        }
        entry.notify.notify_waiters();
        entry.notify.notify_one();
        let state =
            entry_state_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        if let Err(error) =
            persist_task_state(&self.inner.host()?, callback_context(context), &state).await
        {
            lock_state(&entry)?.storage_warning = Some(format!(
                "cancellation outcome known; persistence failed: {}",
                error.failure.user.fallback
            ));
        }
        Ok(task_output(
            "Cancel task",
            format!(
                "Cancellation {} for task '{}'.",
                if response.accepted {
                    "accepted"
                } else {
                    "not accepted"
                },
                input.task_id
            ),
            vec![entry_state_for_parent(
                &self.tasks,
                input.task_id.as_str(),
                context.session_id,
            )?],
            !response.accepted,
        ))
    }

    #[tool(
        tags(subtask, mutate, task),
        summary = "Send additional guidance to a running delegated task.",
        translations(
            locale("zh-CN", summary = "向正在运行的已委派任务发送补充指导。"),
            locale("zh-TW", summary = "向執行中的委派工作傳送補充指引。"),
            locale("ja-JP", summary = "実行中の委任タスクに追加の指示を送ります。"),
            locale("ko-KR", summary = "실행 중인 위임 작업에 추가 지침을 보냅니다."),
            locale(
                "fr-FR",
                summary = "Envoyer des consignes supplémentaires à une tâche déléguée en cours."
            ),
            locale(
                "de-DE",
                summary = "Einer laufenden delegierten Aufgabe zusätzliche Anweisungen senden."
            ),
            locale(
                "es-ES",
                summary = "Envía instrucciones adicionales a una tarea delegada en curso."
            ),
            locale("hi-IN", summary = "चल रहे सौंपे गए कार्य को अतिरिक्त निर्देश भेजें।"),
            locale("ar-SA", summary = "أرسل إرشادات إضافية إلى مهمة مفوّضة قيد التشغيل."),
            locale(
                "pt-BR",
                summary = "Envie orientações adicionais para uma tarefa delegada em andamento."
            )
        )
    )]
    async fn message(
        &self,
        input: &TaskMessageInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let state =
            entry_state_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        if is_terminal(&state.status) {
            return Err(agena_plugin_host::PluginError::invalid_params(format!(
                "task '{}' is terminal; use tasks.followup to resume it",
                input.task_id
            )));
        }
        self.inner
            .host()?
            .message_subtask(MessageSubtaskRequest {
                parent_session_id: Some(state.parent_session_id),
                task_id: input.task_id.clone(),
                message: input.message.clone(),
            })
            .await?;
        Ok(task_output(
            "Send task message",
            format!("Guidance delivered to task '{}'.", input.task_id),
            vec![state],
            false,
        ))
    }

    #[tool(
        tags(subtask, mutate, task),
        summary = "Resume a terminal delegated task with a follow-up prompt.",
        translations(
            locale("zh-CN", summary = "使用后续提示恢复已结束的已委派任务。"),
            locale("zh-TW", summary = "使用後續提示重新啟動已結束的委派工作。"),
            locale(
                "ja-JP",
                summary = "追加のプロンプトを使って、終了した委任タスクを再開します。"
            ),
            locale(
                "ko-KR",
                summary = "후속 프롬프트로 종료된 위임 작업을 다시 시작합니다."
            ),
            locale(
                "fr-FR",
                summary = "Reprendre une tâche déléguée terminée avec une nouvelle consigne."
            ),
            locale(
                "de-DE",
                summary = "Eine beendete delegierte Aufgabe mit einer Folgeanweisung fortsetzen."
            ),
            locale(
                "es-ES",
                summary = "Reanuda una tarea delegada finalizada con una nueva indicación."
            ),
            locale("hi-IN", summary = "आगे का प्रॉम्प्ट देकर समाप्त सौंपे गए कार्य को फिर शुरू करें।"),
            locale("ar-SA", summary = "استأنف مهمة مفوّضة انتهت باستخدام طلب متابعة."),
            locale(
                "pt-BR",
                summary = "Retome uma tarefa delegada encerrada com uma nova solicitação."
            )
        )
    )]
    async fn followup(
        &self,
        input: &TaskFollowupInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let _admission = self.admission.lock().await;
        let entry = task_entry_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        let mut state = recover_task_state(&entry).clone();
        if !is_terminal(&state.status) {
            return Err(agena_plugin_host::PluginError::invalid_params(
                "task is not terminal; send guidance with tasks.message",
            ));
        }
        state.run_epoch = state.run_epoch.checked_add(1).ok_or_else(|| {
            agena_plugin_host::PluginError::invalid_params("task epoch exhausted")
        })?;
        state.status = "running".into();
        state.prompt = input.prompt.clone();
        state.started_at_ms = chrono::Utc::now().timestamp_millis();
        state.finished_at_ms = None;
        state.response = None;
        state.error = None;
        state.storage_warning = None;
        state.budget_exceeded = false;
        state.timeout_ms = input.timeout_ms.or(state.timeout_ms);
        state.max_tokens = input.max_tokens.or(state.max_tokens);
        state.max_cost_microusd = input.max_cost_microusd.or(state.max_cost_microusd);
        let request = RunSubtaskRequest {
            parent_session_id: Some(state.parent_session_id),
            run_in_background: true,
            launch_call_id: Some(context.call_id),
            description: state.description.clone(),
            prompt: state.prompt.clone(),
            commands: None,
            task_id: Some(state.task_id.clone()),
            selection: state.selection.clone(),
            timeout_ms: state.timeout_ms,
            max_tokens: state.max_tokens,
            max_cost_microusd: state.max_cost_microusd,
        };
        let reservation = self.reserve_task(state.clone(), true)?;
        let host = self.inner.host()?;
        persist_task_state(&host, callback_context(context), &state).await?;
        let entry = reservation.commit();
        spawn_task(
            host,
            Arc::clone(&entry),
            request,
            Arc::clone(&self.admission),
            Arc::clone(&self.tasks),
        );
        Ok(task_output(
            "Follow up task",
            format!("Resumed task '{}' with a follow-up prompt.", input.task_id),
            vec![entry_state_for_parent(
                &self.tasks,
                input.task_id.as_str(),
                context.session_id,
            )?],
            false,
        ))
    }

    /// Background tasks are attached to their parent session by default. When
    /// the parent ends, request cancellation of every nonterminal child rather
    /// than leaving unowned provider work running. The child session remains
    /// persisted for audit and `tasks.output`; its normal completion path
    /// writes the final task record if it can observe cancellation.
    #[hook(session.end)]
    async fn session_end(&self, input: SessionEndInput) -> SdkResult<()> {
        let entries = self
            .tasks
            .lock()
            .map_err(|_| agena_plugin_host::PluginError::internal("tasks registry lock poisoned"))?
            .values()
            .filter_map(|entry| {
                let state = recover_task_state(entry).clone();
                (state.parent_session_id == input.session_id && !is_terminal(&state.status))
                    .then(|| (Arc::clone(entry), state))
            })
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return Ok(());
        }
        let host = self.inner.host()?;
        let callback = HostCallbackContext {
            plugin_id: Some(TASKS_PLUGIN_ID.to_string()),
            session_id: Some(input.session_id),
            ..current_host_callback_context().unwrap_or_default()
        };
        for (entry, state) in entries {
            if let Err(error) = run_in_host_callback_context(
                callback.clone(),
                host.cancel_subtask(CancelSubtaskRequest {
                    parent_session_id: Some(input.session_id),
                    task_id: state.task_id.clone(),
                }),
            )
            .await
            {
                tracing::warn!(
                    target: "agena_tasks",
                    task_id = %state.task_id,
                    parent_session_id = input.session_id,
                    %error,
                    "failed to request child cancellation while parent session ended"
                );
                continue;
            }
            let mut mutable = recover_task_state(&entry);
            if !is_terminal(&mutable.status) {
                mutable.status = "cancelling".to_string();
                mutable.error = Some("parent session ended; cancellation requested".to_string());
            }
            drop(mutable);
            entry.notify.notify_waiters();
            entry.notify.notify_one();
        }
        Ok(())
    }

    /// Rebuild task handles for the invoking parent session from plugin-private
    /// durable storage. Keys are namespaced by parent session id so background
    /// task fibers never need to retain/replay a foreground session authority.
    /// The child session transcript remains
    /// the source of truth for output; this registry supplies task metadata
    /// after plugin reconstruction.
    ///
    /// A nonterminal persisted handle becomes `interrupted`, rather than
    /// automatically replaying its prompt. The child session might already
    /// contain that prompt when a process stopped before acknowledgement, so
    /// automatic restart could duplicate side effects. `tasks.output` and an
    /// explicit `tasks.followup` remain available for recovery.
    async fn hydrate_session_tasks(&self, context: &ToolInvokeContext<'_>) -> SdkResult<()> {
        let _admission = self.admission.lock().await;
        let host = self.inner.host()?;
        let records = host
            .storage_list(HostStorageListRequest {
                scope: HostStorageScope::Global,
                visibility: Default::default(),
                namespace: Some(TASK_STORAGE_NAMESPACE.to_string()),
                prefix: Some(task_storage_prefix(context.session_id)),
            })
            .await?;
        for record in records.records {
            let response = host
                .storage_get(HostStorageGetRequest {
                    scope: HostStorageScope::Global,
                    visibility: Default::default(),
                    namespace: TASK_STORAGE_NAMESPACE.to_string(),
                    key: record.key,
                })
                .await?;
            let Some(value) = response.value else {
                continue;
            };
            let mut state: AsyncTaskState = serde_json::from_str(&value).map_err(|error| {
                agena_plugin_host::PluginError::internal(format!(
                    "invalid persisted task registry entry: {error}"
                ))
            })?;
            if state.parent_session_id != context.session_id {
                continue;
            }
            let exists = self
                .tasks
                .lock()
                .map_err(|_| {
                    agena_plugin_host::PluginError::internal("tasks registry lock poisoned")
                })?
                .contains_key(&task_storage_key(&state));
            if exists {
                continue;
            }
            if !is_terminal(&state.status) {
                state.status = "interrupted".to_string();
                state.finished_at_ms = Some(chrono::Utc::now().timestamp_millis());
                state.error = Some(
                    "Agena restarted before this delegated task reported a terminal result; inspect output and use tasks.followup for explicit recovery."
                        .to_string(),
                );
                persist_task_state(&host, callback_context(context), &state).await?;
            }
            let entry = Arc::new(AsyncTaskEntry {
                state: Mutex::new(state.clone()),
                notify: Arc::new(Notify::new()),
            });
            self.tasks
                .lock()
                .map_err(|_| {
                    agena_plugin_host::PluginError::internal("tasks registry lock poisoned")
                })?
                .insert(task_storage_key(&state), entry);
        }
        Ok(())
    }
}

fn callback_context(context: &ToolInvokeContext<'_>) -> HostCallbackContext {
    HostCallbackContext {
        plugin_id: Some(TASKS_PLUGIN_ID.to_string()),
        session_id: Some(context.session_id),
        call_id: Some(context.call_id),
        workspace_root: Some(context.workspace_root.to_string()),
        tool_name: Some(context.tool_name.to_string()),
        ..current_host_callback_context().unwrap_or_default()
    }
}

fn detached_task_context() -> HostCallbackContext {
    HostCallbackContext {
        plugin_id: Some(TASKS_PLUGIN_ID.to_string()),
        ..HostCallbackContext::default()
    }
}

fn task_storage_prefix(parent_session_id: i64) -> String {
    format!("{parent_session_id}/")
}

fn task_storage_key(state: &AsyncTaskState) -> String {
    format!(
        "{}{task_id}",
        task_storage_prefix(state.parent_session_id),
        task_id = state.task_id
    )
}

async fn persist_task_state(
    host: &Arc<dyn HostClient>,
    context: HostCallbackContext,
    state: &AsyncTaskState,
) -> SdkResult<()> {
    let value = serde_json::to_string(state).map_err(|error| {
        agena_plugin_host::PluginError::internal(
            agena_failure::diagnostic::format_error_chain_with_context(
                "serialize background task state",
                &error,
            ),
        )
    })?;
    run_in_host_callback_context(
        context,
        host.storage_set(HostStorageSetRequest {
            scope: HostStorageScope::Global,
            visibility: Default::default(),
            namespace: TASK_STORAGE_NAMESPACE.to_string(),
            key: task_storage_key(state),
            value,
        }),
    )
    .await
}

fn spawn_task(
    host: Arc<dyn HostClient>,
    entry: Arc<AsyncTaskEntry>,
    request: RunSubtaskRequest,
    admission: Arc<tokio::sync::Mutex<()>>,
    tasks: Arc<Mutex<BTreeMap<String, Arc<AsyncTaskEntry>>>>,
) {
    tokio::spawn(async move {
        // Detached work deliberately drops session/call/tool authority. The
        // subtask request carries its parent session explicitly and task state
        // lives in plugin-private global storage, so the background fiber only
        // needs the plugin identity granted by ScopedHostClient/transport.
        let context = detached_task_context();
        let result = run_in_host_callback_context(context.clone(), host.run_subtask(request)).await;
        let _admission = admission.lock().await;
        let key = task_storage_key(&recover_task_state(&entry));
        if tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
            .is_none_or(|current| !Arc::ptr_eq(current, &entry))
        {
            return;
        }
        let persisted = {
            let mut state = recover_task_state(&entry);
            state.finished_at_ms = Some(chrono::Utc::now().timestamp_millis());
            match result {
                Ok(response) => {
                    state.status = status_name(response.status).to_string();
                    state.error = response
                        .problem
                        .as_ref()
                        .map(|failure| failure.user.fallback.clone());
                    state.budget_exceeded = response.budget_exceeded;
                    state.response = Some(response);
                }
                Err(error) => {
                    tracing::error!(
                        target: "agena_tasks",
                        diagnostic = %error.diagnostic.message,
                        "background task execution failed"
                    );
                    state.status = "failed".to_string();
                    state.error = Some(error.failure.user.fallback.clone());
                }
            }
            state.clone()
        };
        if let Err(error) = persist_task_state(&host, context, &persisted).await {
            recover_task_state(&entry).storage_warning=Some("task finished, but saving its terminal record failed; inspect child transcript before retrying".into());
            tracing::warn!(
                target: "agena_tasks",
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "persist completed background task state",
                    &error,
                ),
                task_id = %persisted.task_id,
                "failed to persist terminal task state"
            );
        }
        entry.notify.notify_waiters();
        // Preserve one permit for a waiter that checked state immediately
        // before this completion and had not yet polled its notification.
        entry.notify.notify_one();
    });
}

fn status_name(status: RunSubtaskStatus) -> &'static str {
    match status {
        RunSubtaskStatus::Created => "created",
        RunSubtaskStatus::Running => "running",
        RunSubtaskStatus::Completed => "completed",
        RunSubtaskStatus::Failed => "failed",
        RunSubtaskStatus::Cancelled => "cancelled",
        RunSubtaskStatus::TimedOut => "timed_out",
        RunSubtaskStatus::Interrupted => "interrupted",
    }
}

fn is_terminal(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "cancelled" | "timed_out" | "interrupted"
    )
}

fn entry_state_for_parent(
    tasks: &Mutex<BTreeMap<String, Arc<AsyncTaskEntry>>>,
    task_id: &str,
    parent_session_id: i64,
) -> SdkResult<AsyncTaskState> {
    let entry = task_entry_for_parent(tasks, task_id, parent_session_id)?;
    Ok(recover_task_state(&entry).clone())
}
fn task_entry_for_parent(
    tasks: &Mutex<BTreeMap<String, Arc<AsyncTaskEntry>>>,
    task_id: &str,
    parent_session_id: i64,
) -> SdkResult<Arc<AsyncTaskEntry>> {
    tasks
        .lock()
        .map_err(|_| agena_plugin_host::PluginError::internal("task registry poisoned"))?
        .get(&format!(
            "{}{task_id}",
            task_storage_prefix(parent_session_id)
        ))
        .cloned()
        .ok_or_else(|| {
            agena_plugin_host::PluginError::invalid_params(format!("unknown task '{task_id}'"))
        })
}

fn lock_state(entry: &AsyncTaskEntry) -> SdkResult<std::sync::MutexGuard<'_, AsyncTaskState>> {
    Ok(recover_task_state(entry))
}

fn recover_task_state(entry: &AsyncTaskEntry) -> std::sync::MutexGuard<'_, AsyncTaskState> {
    match entry.state.lock() {
        Ok(state) => state,
        Err(error) => {
            tracing::error!(
                operation = "access background task state",
                error = %error,
                "recovering poisoned background task state lock"
            );
            error.into_inner()
        }
    }
}

fn task_output(
    title: impl Into<String>,
    text: impl Into<String>,
    tasks: Vec<AsyncTaskState>,
    timed_out: bool,
) -> ToolInvokeOutput {
    let summary = if timed_out {
        format!("Timed out · {} tasks", tasks.len())
    } else if let [task] = tasks.as_slice() {
        task.status.clone()
    } else {
        let terminal = tasks
            .iter()
            .filter(|task| is_terminal(task.status.as_str()))
            .count();
        format!("{} tasks · {terminal} terminal", tasks.len())
    };
    ToolInvokeOutput::from_parts(
        title,
        summary,
        text,
        Some(serde_json::json!({ "tasks": tasks, "timed_out": timed_out })),
        BTreeMap::from([
            ("task_count".to_string(), tasks.len().to_string()),
            ("timed_out".to_string(), timed_out.to_string()),
        ]),
        Vec::new(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::part::TaskToolInput;
    use agena_plugin_host::sdk::Plugin;

    use super::{AsyncTaskEntry, AsyncTaskState, TasksPlugin, lock_state};

    #[test]
    fn task_contract_exposes_terminal_task_fields() {
        let manifest = TasksPlugin::new().manifest();
        assert_eq!(
            manifest.summary_for_locale("zh-CN"),
            Some("委派子任务编排工具。")
        );
        let tool = manifest.tools.first().expect("task tool");
        assert_eq!(tool.name, "run");
        let translated_tools = manifest
            .tools
            .iter()
            .filter(|tool| tool.docs.summary_for_locale("zh-CN") != tool.docs.summary.as_deref())
            .count();
        assert_eq!(
            translated_tools,
            manifest.tools.len(),
            "every task tool needs a Chinese UI summary"
        );
        assert!(tool.docs.help_for_locale("zh-CN").is_some());
        for locale in [
            "zh-CN", "zh-TW", "ja-JP", "ko-KR", "fr-FR", "de-DE", "es-ES", "hi-IN", "ar-SA",
            "pt-BR",
        ] {
            assert_ne!(
                manifest.summary_for_locale(locale),
                manifest.summary.as_deref(),
                "task plugin summary needs a native {locale} translation"
            );
            assert!(
                manifest.tools.iter().all(|tool| {
                    tool.docs.summary_for_locale(locale) != tool.docs.summary.as_deref()
                }),
                "every task tool summary needs a native {locale} translation"
            );
        }
        let schema = &tool.contract.input_schema;
        assert!(schema.pointer("/properties/profile").is_none());
        assert!(schema.pointer("/properties/selection").is_some());
        assert!(schema.pointer("/properties/commands").is_some());
        assert!(schema.pointer("/properties/subagent_type").is_none());
        assert!(schema.pointer("/properties/command").is_none());
        assert_eq!(
            schema.pointer("/properties/timeout_ms/minimum"),
            Some(&serde_json::json!(1))
        );
        assert_eq!(
            schema.pointer("/properties/max_tokens/minimum"),
            Some(&serde_json::json!(1))
        );
        assert_eq!(
            schema.pointer("/properties/max_cost_microusd/minimum"),
            Some(&serde_json::json!(1))
        );
        let names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "run", "list", "get", "output", "cancel", "message", "followup"
            ]
        );
        for name in ["output", "cancel", "message", "followup"] {
            assert!(
                manifest.tools.iter().any(|tool| tool.name == name),
                "missing task lifecycle tool `{name}`"
            );
        }
    }

    #[test]
    fn task_input_rejects_zero_timeout_and_unknown_fields() {
        let valid = serde_json::json!({
            "description": "verify",
            "prompt": "run the checks",
            "commands": ["verify", "security-review"],
            "timeout_ms": 1
        });
        assert!(TaskToolInput::parse_input(valid).is_ok());

        for invalid in [
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "timeout_ms": 0
            }),
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "max_tokens": 0
            }),
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "max_cost_microusd": 0
            }),
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "task_id": "   "
            }),
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "profile": "verify",
                "subagent_type": "verify"
            }),
        ] {
            assert!(TaskToolInput::parse_input(invalid).is_err());
        }
    }

    #[tokio::test]
    async fn task_notification_preserves_a_completion_wakeup() {
        let entry = Arc::new(AsyncTaskEntry {
            state: std::sync::Mutex::new(AsyncTaskState {
                run_epoch: 1,
                storage_warning: None,
                task_id: "task_wait".to_string(),
                parent_session_id: 7,
                description: "wait".to_string(),
                prompt: "wait".to_string(),
                status: "running".to_string(),
                started_at_ms: 1,
                finished_at_ms: None,
                response: None,
                error: None,
                selection: None,
                timeout_ms: None,
                max_tokens: None,
                max_cost_microusd: None,
                budget_exceeded: false,
            }),
            notify: Arc::new(tokio::sync::Notify::new()),
        });
        let waiter = {
            let entry = Arc::clone(&entry);
            tokio::spawn(async move {
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    Arc::clone(&entry.notify).notified_owned(),
                )
                .await
            })
        };
        tokio::task::yield_now().await;
        lock_state(&entry).expect("task state").status = "completed".to_string();
        entry.notify.notify_waiters();
        entry.notify.notify_one();

        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("waiter woke")
            .expect("waiter joined")
            .expect("notification completed");
    }
}
