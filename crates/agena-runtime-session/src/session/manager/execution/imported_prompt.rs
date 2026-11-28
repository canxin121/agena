use super::*;
use super::super::media::UserMediaSnapshots;
use crate::{SessionExecutionCommandOutcome, session::model::SessionExecutionContext};
use agena_domain::{
    AuthorKind, ExecutionLifecycle, LocalSurface, MessageAuthor, MessageOrigin, SystemAuthorKind,
};
use agena_plugin_host::{
    UserPromptSubmitInput,
    host::{ImportedPromptMediaReader, ImportedPromptSubmit, decode_plugin_command_input},
    sdk::{
        CommandHostEffect, CommandInvokeInput, CommandStatus, PluginErrorKind, PluginKey,
        cancellation::run_with_cancellation,
    },
};
use agena_session_api::commands::SessionPromptCommand;
use futures_util::future::BoxFuture;
use serde::Deserialize;

mod media;
use media::{PreparedPromptMedia, PromptMedia};

pub(super) struct PendingSubmission {
    pub session: SessionId,
    pub state: Arc<ExecutionState>,
    pub context: SessionExecutionContext,
    pub options: SessionRunOptions,
    pub message: UserMessageInput,
    pub plugin: Option<PluginSubmission>,
    pub turn: Option<ExecutionView>,
    pub command: Option<SessionPromptCommand>,
    pub literal_leading_slash: bool,
    pub snapshots: UserMediaSnapshots,
    pub wait: bool,
    pub ingress_author: Option<MessageAuthor>,
    pub ingress_origin: Option<MessageOrigin>,
    pub send_id: RequestId,
    pub permit: OwnedSemaphorePermit,
}

pub(super) struct PluginSubmission {
    pub key: PluginKey,
    pub join_turn: bool,
}

pub(super) struct PromptAdmission {
    pub execution: Option<SessionExecutionCommandOutcome>,
    pub answer: Value,
}

impl PromptAdmission {
    fn accepted(
        execution: Option<SessionExecutionCommandOutcome>,
        draft: PromptDraft,
        origin: Value,
    ) -> Self {
        let mut answer = json!({"text":draft.text, "origin":origin});
        if !draft.context.is_empty() {
            answer["context"] = json!(draft.context);
        }
        Self { execution, answer }
    }
}

#[derive(Deserialize)]
struct PromptDraft {
    text: String,
    #[serde(default)]
    context: Vec<String>,
}

struct PromptCommand {
    plugin: PluginKey,
    name: String,
    line: String,
    input: CommandInvokeInput,
}

impl PromptCommand {
    fn resolve(
        request: SessionPromptCommand,
        session: SessionId,
        executor: &ToolExecutor,
    ) -> Result<Self, AppError> {
        let SessionPromptCommand {
            plugin_id,
            command_id,
            args: raw,
        } = request;
        if raw.len() > 16 * 1024 {
            return Err(AppError::Config(
                "prompt command arguments exceed 16 KiB".into(),
            ));
        }
        let unavailable =
            || AppError::Config("The prompt command is unavailable in this session".into());
        let plugin: PluginKey = plugin_id.parse().map_err(|_| unavailable())?;
        let host = executor.plugin_manager();
        let name = host
            .imported_prompt_command_name(&plugin, &command_id)
            .filter(|_| executor.plugin_is_enabled(&plugin))
            .ok_or_else(unavailable)?;
        let definition = host
            .resolve_command_for_session(Some(session.get()), &plugin_id, &command_id)
            .ok_or_else(unavailable)?;
        let slash = definition.slash.clone().ok_or_else(unavailable)?;
        let input = decode_plugin_command_input(&definition, json!({}), &raw)
            .map_err(|error| AppError::Config(error.diagnostic_message().to_owned()))?;
        let line = if raw.is_empty() {
            slash.clone()
        } else {
            format!("{slash} {raw}")
        };
        Ok(Self {
            plugin,
            name,
            line,
            input: CommandInvokeInput {
                session_id: Some(session.get()),
                call_id: None,
                workspace_root: Some(executor.workspace_root().to_string_lossy().into_owned()),
                command_id,
                slash: Some(slash),
                raw,
                input,
                prompt_context: String::new(),
                cordis_attachments: None,
            },
        })
    }

    async fn render(self, executor: &ToolExecutor) -> Result<String, AppError> {
        let result = executor
            .plugin_manager()
            .invoke_imported_prompt_command(&self.plugin, self.input, |key| {
                executor.plugin_is_enabled(key)
            })
            .await
            .map_err(|error| match error.kind {
                PluginErrorKind::Cancelled => AppError::Cancelled,
                _ => AppError::Config(error.failure.user.fallback),
            })?;
        if result.status != CommandStatus::Succeeded {
            return Err(AppError::Config(result.detail.unwrap_or(result.summary)));
        }
        let mut effects = result.effects.into_iter();
        match (effects.next(), effects.next()) {
            (Some(CommandHostEffect::InsertPrompt { prompt }), None) => Ok(prompt),
            _ => Err(AppError::Internal(
                "A prompt command did not return a prompt".into(),
            )),
        }
    }
}

impl SessionManager {
    pub async fn submit_imported_plugin_prompt(
        &self,
        id: i64,
        key: PluginKey,
        name: String,
        request: agena_plugin_host::sdk::host_api::ImportedPluginPromptRequest,
    ) -> Result<Value, AppError> {
        if request.text.trim().is_empty() || request.text.len() > 256 * 1024 {
            return Err(AppError::Config(
                "A plugin prompt requires text within 256 KiB".into(),
            ));
        }
        let session = session_id(id)?;
        let metadata = read_with_busy_retry(session, "metadata", || {
            self.kernel().reader.metadata(session)
        })
        .await?;
        let options = self
            .options_for_session(session, &metadata.selection.clone().unwrap_or_default())
            .await?;
        let author = MessageAuthor::plugin(&key.to_string(), name, request.as_user);
        let submission = crate::SessionUserRunRequest::new(
            id,
            options,
            ComposerDocument(vec![ComposerNode::Text { text: request.text }]),
        );
        self.admit_user_message(
            submission,
            Some(author),
            None,
            Some(PluginSubmission {
                key,
                join_turn: !request.wait,
            }),
            UserMediaSnapshots::new(),
        )
        .await
        .map(|admission| admission.answer)
    }

    pub(super) fn submit_prompt(
        &self,
        mut submission: PendingSubmission,
    ) -> BoxFuture<'_, Result<PromptAdmission, AppError>> {
        async move {
            let original = submitted_text(&submission.message.document);
            let executor = submission
                .state
                .executor
                .for_session_resource_context(&submission.context);
            let executor = &executor;
            let plugin_host = executor.plugin_manager();
            let project = submission.context.owner.is_none()
                && submission.context.channel_authority.is_none()
                && !executor.is_global();
            if submission
                .plugin
                .as_ref()
                .is_some_and(|plugin| !project || !executor.plugin_is_enabled(&plugin.key))
            {
                return Err(AppError::Config(
                    "This plugin cannot submit prompts in the current session".into(),
                ));
            }
            if submission
                .plugin
                .as_ref()
                .is_some_and(|plugin| plugin.join_turn)
                && let Some(id) = self.running_execution(submission.session).await?
            {
                let view = read_with_busy_retry(submission.session, "execution", || {
                    self.kernel().reader.execution(submission.session, id)
                })
                .await?;
                if view.phase == ExecutionPhase::Running
                    && view.purpose != ExecutionPurpose::Compaction
                {
                    submission.turn = Some((*view).clone());
                }
            }
            let command = if let Some(request) = submission.command.take() {
                if !project {
                    return Err(AppError::Config(
                        "Prompt commands require a project session".into(),
                    ));
                }
                Some(PromptCommand::resolve(
                    request,
                    submission.session,
                    executor,
                )?)
            } else {
                None
            };
            let text = command
                .as_ref()
                .map_or(original.as_str(), |command| &command.line);
            let author = submission
                .message
                .author
                .as_ref()
                .or(submission.ingress_author.as_ref());
            let origin = prompt_origin(
                author,
                submission
                    .message
                    .origin
                    .as_ref()
                    .or(submission.ingress_origin.as_ref()),
            );
            if !project || !plugin_host.has_imported_prompt_submit_hooks() {
                let draft = PromptDraft {
                    text: text.to_owned(),
                    context: Vec::new(),
                };
                return self
                    .commit_user_prompt(submission, executor, &original, draft, command, None)
                    .await
                    .map(|(outcome, draft)| PromptAdmission::accepted(outcome, draft, origin));
            }

            let session = submission.session.get();
            let workspace = executor.workspace_root().to_string_lossy();
            let model = submission.options.model.model_id.to_string();
            let mut input = self
                .prompt_submit_event(
                    &submission,
                    text,
                    origin,
                    author.is_some_and(|author| author.kind.is_system()),
                )
                .await;
            let media = PromptMedia::for_submission(self, &submission);
            if let Some(media) = &media {
                input["attachments"] = json!(media.metadata());
            }
            let media = media.as_ref();
            let read_media = |id, offset| -> BoxFuture<'_, Value> {
                async move {
                    match media {
                        Some(media) => media.read(id, offset).await,
                        None => json!({"deny":"This submission has no prompt attachments"}),
                    }
                }
                .boxed()
            };
            if let Some(command) = &command {
                input["command"] = json!({
                    "name":command.name,
                    "args":command.input.input["args"].as_str().unwrap_or_default(),
                });
            }
            let mut committed = None;
            let receipt = &mut committed;
            let answer = run_with_cancellation(self.shutdown.child_token(), async {
                Ok(plugin_host
                    .run_imported_prompt_submit(
                        ImportedPromptSubmit {
                            session_id: session,
                            workspace_root: &workspace,
                            model: &model,
                            sandbox_enabled: executor.sandbox_enabled(),
                            input,
                            media: media.map(|_| &read_media as &ImportedPromptMediaReader<'_>),
                        },
                        |key| executor.plugin_is_enabled(key),
                        |mut event| {
                            Box::pin(async move {
                                let origin = event["origin"].take();
                                let result = match serde_json::from_value(event) {
                                    Ok(draft) => {
                                        self.commit_user_prompt(
                                            submission,
                                            executor,
                                            &original,
                                            draft,
                                            command,
                                            media.and_then(PromptMedia::prepared),
                                        )
                                        .await
                                    }
                                    Err(error) => Err(AppError::SerdeJson(error)),
                                }
                                .map(|(outcome, draft)| {
                                    PromptAdmission::accepted(outcome, draft, origin)
                                });
                                let answer = match &result {
                                    Ok(admission) => admission.answer.clone(),
                                    Err(error) => json!({"drop":error.failure().user.fallback}),
                                };
                                *receipt = Some(result);
                                answer
                            })
                        },
                    )
                    .await)
            })
            .await;

            if let Some(result) = committed {
                return result;
            }
            let answer = answer.map_err(|_| AppError::Cancelled)?;
            Err(AppError::PromptSubmitBlocked(
                answer["drop"]
                    .as_str()
                    .filter(|reason| !reason.trim().is_empty())
                    .unwrap_or("The hook ended the submission before forwarding it.")
                    .to_owned(),
            ))
        }
        .boxed()
    }

    async fn prompt_submit_event(
        &self,
        submission: &PendingSubmission,
        text: &str,
        origin: Value,
        meta: bool,
    ) -> Value {
        let mut event = json!({
            "text":text,
            "wait":submission.wait,
            "origin":origin,
        });
        if meta {
            event["isMeta"] = json!(true);
        }
        if let Some(turn) = &submission.turn {
            event["turnId"] = json!(turn.id);
        } else if submission.plugin.is_none()
            && let Some(ExecutionLifecycle::Active {
                execution_id,
                phase,
            }) = self.active_execution(submission.session.get()).await
            && phase != agena_domain::ExecutionPhase::Starting
        {
            event["turnId"] = json!(execution_id);
        }
        event
    }

    // next(e) awaits the durable admission of the input. Keep this future
    // outside the VM callback's stack and keep the submission lock held until
    // its owner has the result.
    fn commit_user_prompt<'a>(
        &'a self,
        submission: PendingSubmission,
        executor: &'a ToolExecutor,
        original: &'a str,
        mut draft: PromptDraft,
        command: Option<PromptCommand>,
        prepared_media: Option<&'a PreparedPromptMedia>,
    ) -> BoxFuture<'a, Result<(Option<SessionExecutionCommandOutcome>, PromptDraft), AppError>>
    {
        async move {
            let PendingSubmission {
                session,
                state,
                context,
                options,
                mut message,
                plugin,
                turn,
                snapshots,
                literal_leading_slash,
                ingress_author,
                ingress_origin,
                send_id,
                permit,
                ..
            } = submission;
            if !draft.text.is_empty() {
                let settings = executor
                    .plugin_manager()
                    .dispatch_user_prompt_submit_with_context(
                        UserPromptSubmitInput {
                            session_id: session.get(),
                            prompt: std::mem::take(&mut draft.text),
                        },
                        Some(self.shutdown.child_token()),
                        |key| executor.plugin_is_enabled(key),
                    )
                    .await
                    .map_err(|error| match error.kind {
                        PluginErrorKind::PolicyDenied => {
                            AppError::PromptSubmitBlocked(error.failure.user.fallback)
                        }
                        PluginErrorKind::Cancelled => AppError::Cancelled,
                        _ => AppError::Internal(error.diagnostic_message().to_owned()),
                    })?;
                draft.text = settings.prompt;
                draft.context.extend(settings.additional_context);
            }
            let direct_prompt = plugin.is_none() && command.is_none();
            let rendered = match command {
                Some(command) if draft.text == command.line => {
                    Some(command.render(executor).await?)
                }
                _ => None,
            };
            let text = rendered.as_deref().unwrap_or(&draft.text);
            let (mut document, blobs) = match prepared_media {
                Some((document, blobs)) => (document.clone(), Some(blobs.clone())),
                None => (message.document, None),
            };
            let operator = context.owner.is_none()
                && context.channel_authority.is_none()
                && message.author.as_ref().or(ingress_author.as_ref())
                    .is_some_and(MessageAuthor::is_instance_operator)
                && message.origin.as_ref().or(ingress_origin.as_ref())
                    .is_some_and(|origin| origin.platform.is_local());
            use super::super::imported_skills::{selected_skill_context, typed_skill_names};
            let mut typed = if direct_prompt && operator && !executor.is_global() {
                typed_skill_names(&document, literal_leading_slash)
            } else {
                Vec::new()
            };
            update_document_text(&mut document, original, text)?;
            if !typed.is_empty() && text != original {
                // Hooks may remove a user gesture. Added text has no user
                // authority to invoke another skill.
                let retained = typed_skill_names(&document, false);
                typed.retain(|name| retained.contains(name));
            }
            let selected = selected_skill_context(
                executor,
                &mut document,
                operator,
                &typed,
            )
            .await?;
            draft.context = bound_context(draft.context, &selected);
            let (document, blobs) = match blobs {
                Some(blobs) => (document, blobs),
                None => {
                    self.prepare_user_media_with_state(
                        session,
                        &options.model,
                        document,
                        send_id,
                        state,
                        Some(context),
                        Some(&snapshots),
                    )
                    .await?
                }
            };
            message.document = document;
            message.model_context.clone_from(&draft.context);
            message.model_context.extend(selected);
            if let (Some(turn), Some(author)) = (turn, message.author.as_ref()) {
                let request = request_key("plugin-prompt", &send_id.to_string());
                let key = format!("plugin-prompt:{request}");
                let payload = json!({
                    "kind": "plugin_prompt",
                    "generation": turn.generation,
                    "text": draft.text,
                    "author": author,
                    "model_context": draft.context,
                });
                match persist(|| {
                    self.kernel().executor.deliver_to_execution(
                        request,
                        session,
                        turn.id,
                        key.clone(),
                        payload.clone(),
                    )
                })
                .await
                {
                    Ok(_) => return Ok((None, draft)),
                    Err(KernelError::Conflict(_) | KernelError::NotFound(_)) => {}
                    Err(error) => return Err(kernel_error(error)),
                }
            }
            self.persist_submission_selection(session.get(), options.clone())
                .await?;
            let receipt = persist(|| {
                self.kernel()
                    .sessions
                    .send_user_message(send_id, session, message.clone(), &blobs)
            })
            .await
            .map_err(kernel_error)?;
            let outcome = execution_outcome(&receipt.value);
            self.supervise(receipt.value, options, false, permit)
                .await?;
            self.schedule_title(session);
            Ok((Some(outcome), draft))
        }
        .boxed()
    }
}

fn submitted_text(document: &ComposerDocument) -> String {
    document
        .0
        .iter()
        .filter_map(|node| match node {
            ComposerNode::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn update_document_text(
    document: &mut ComposerDocument,
    original: &str,
    text: &str,
) -> Result<(), AppError> {
    if text == original {
        return Ok(());
    }
    if text.len() > 256 * 1024 {
        return Err(AppError::Config(
            "rewritten prompt exceeds the input byte budget".into(),
        ));
    }
    let mut placed = text.is_empty();
    document.0.retain_mut(|node| match node {
        ComposerNode::Text { text: previous } if !placed => {
            *previous = text.to_owned();
            placed = true;
            true
        }
        ComposerNode::Text { .. } => false,
        _ => true,
    });
    if !placed {
        document.0.insert(
            0,
            ComposerNode::Text {
                text: text.to_owned(),
            },
        );
    }
    if document.is_empty() {
        return Err(AppError::Config(
            "the rewritten message must contain text or an attachment".into(),
        ));
    }
    Ok(())
}

fn bound_context(context: Vec<String>, selected: &[String]) -> Vec<String> {
    let mut remaining = MAX_USER_CONTEXT_BYTES.saturating_sub(selected.iter().map(String::len).sum());
    context
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .take(MAX_USER_CONTEXT_BLOCKS.saturating_sub(selected.len()))
        .map_while(|mut text| {
            if remaining == 0 {
                return None;
            }
            text.truncate(
                text.floor_char_boundary(
                    text.len().min(MAX_USER_CONTEXT_BLOCK_BYTES).min(remaining),
                ),
            );
            remaining -= text.len();
            Some(text)
        })
        .filter(|text| !text.is_empty())
        .collect()
}

fn prompt_origin(author: Option<&MessageAuthor>, origin: Option<&MessageOrigin>) -> Value {
    if let Some(author) = author
        && let AuthorKind::Plugin { as_user } = author.kind
    {
        let mut origin = json!({"kind":"plugin", "name":author.display_label()});
        if as_user {
            origin["asUser"] = json!(true);
        }
        return origin;
    }
    let kind = match author.map(|author| &author.kind) {
        Some(AuthorKind::System {
            source: SystemAuthorKind::AutomationReport | SystemAuthorKind::Integration,
        }) => "task-notification",
        Some(AuthorKind::System {
            source: SystemAuthorKind::Subagent,
        }) => "peer-send-message",
        Some(AuthorKind::System {
            source: SystemAuthorKind::Bot,
        }) => "coordinator",
        _ => {
            if let Some(platform) = origin.and_then(|origin| origin.platform.chat_platform()) {
                return json!({"kind":"channel", "server":platform.id});
            }
            match origin.and_then(|origin| origin.platform.local_surface()) {
                Some(LocalSurface::Tui | LocalSurface::Cli) => "composer",
                Some(LocalSurface::Web) => "bridge",
                Some(LocalSurface::Api) => "sdk",
                None => "unclassified",
            }
        }
    };
    json!({"kind":kind})
}
