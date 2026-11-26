use std::collections::BTreeMap;

use agena_domain::{
    EXTERNAL_AGENT_URL_OPEN_OPTION, UserInputReply, UserInputReplyKind, UserInputRequest,
    UserInputSource,
};
use tokio::sync::oneshot;

use super::{
    elicitation::{Form, form_input, url_input},
    *,
};

pub(super) const INPUT_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_INPUT_BYTES: usize = 64 * 1024;
const ID_PREFIX: &str = "acp:";

#[derive(Clone, Default)]
pub(in crate::session::manager) struct Inputs(
    Arc<StdMutex<HashMap<(SessionId, RequestId), Pending>>>,
);

struct Pending {
    request: UserInputRequest,
    response: Option<oneshot::Sender<Value>>,
    retired: Option<oneshot::Receiver<()>>,
    reply: NativeReply,
    scope: ExternalAgentInputScope,
    stop: CancellationToken,
    withdrawal: CancellationToken,
}

impl Pending {
    fn is_waiting(&self, configuration: &SessionConfiguration) -> bool {
        self.response.is_some()
            && !self.stop.is_cancelled()
            && !self.withdrawal.is_cancelled()
            && configuration.external_agent == Some(self.scope.agent)
            && self
                .scope
                .idle_resume
                .as_ref()
                .is_none_or(|resume| configuration.external_agent_resume.as_ref() == Some(resume))
    }
}

#[derive(Clone)]
pub(super) struct Host {
    kernel: KernelPorts,
    inputs: Inputs,
    tasks: tokio_util::task::TaskTracker,
    session: SessionId,
    scope: ExternalAgentInputScope,
    stop: CancellationToken,
}

pub(super) struct NativeInput {
    prompt: crate::part::AskUserToolInput,
    reply: NativeReply,
}

enum NativeReply {
    Form(Form),
    Url,
    Permission(BTreeMap<String, String>),
}

impl NativeInput {
    pub(super) fn elicitation(params: &Value, label: &str) -> Option<Self> {
        if !has_bounded_input(params) {
            return None;
        }
        let (prompt, reply) = match params["mode"].as_str()? {
            "form" => {
                let (prompt, form) = form_input(params, label)?;
                (prompt, NativeReply::Form(form))
            }
            "url" => (url_input(params, label)?, NativeReply::Url),
            _ => return None,
        };
        Some(Self { prompt, reply })
    }

    pub(super) fn permission(params: &Value, label: &str) -> Option<Self> {
        if !has_bounded_input(params) {
            return None;
        }
        let input = permission_input(params, label)?;
        let options = input
            .choices
            .into_iter()
            .map(|choice| (choice.label, choice.id.to_owned()))
            .collect();
        Some(Self {
            prompt: input.prompt,
            reply: NativeReply::Permission(options),
        })
    }
}

fn has_bounded_input(params: &Value) -> bool {
    serde_json::to_vec(params).is_ok_and(|bytes| bytes.len() <= MAX_INPUT_BYTES)
}

impl NativeReply {
    fn cancel(&self) -> Value {
        match self {
            Self::Permission(_) => json!({"outcome": {"outcome": "cancelled"}}),
            _ => json!({"action": "cancel"}),
        }
    }

    fn answer(&self, reply: &UserInputReply) -> Result<Value, AppError> {
        match reply.kind {
            UserInputReplyKind::Timeout => Ok(self.cancel()),
            UserInputReplyKind::Cancel => Ok(match self {
                Self::Permission(_) => self.cancel(),
                _ => json!({"action": "decline"}),
            }),
            UserInputReplyKind::Submit => Ok(match self {
                Self::Form(form) => {
                    json!({"action": "accept", "content": form.content(&reply.answers)?})
                }
                Self::Url => {
                    if first_answer(reply) != Some(EXTERNAL_AGENT_URL_OPEN_OPTION) {
                        return Err(AppError::Config(
                            "Choose the authorization action offered by the agent.".into(),
                        ));
                    }
                    json!({"action": "accept"})
                }
                Self::Permission(choices) => {
                    let choice = first_answer(reply)
                        .and_then(|label| choices.get(label))
                        .ok_or_else(|| {
                            AppError::Config(
                                "Choose a permission option offered by the agent.".into(),
                            )
                        })?;
                    json!({"outcome": {"outcome": "selected", "optionId": choice}})
                }
            }),
        }
    }
}

fn first_answer(reply: &UserInputReply) -> Option<&str> {
    reply
        .answers
        .get("0")
        .and_then(|values| values.first())
        .map(String::as_str)
}

pub(in crate::session::manager) fn input_id(public: &str) -> Option<RequestId> {
    public.strip_prefix(ID_PREFIX)?.parse().ok()
}

impl Host {
    pub(super) fn new(
        manager: &SessionManager,
        session: SessionId,
        agent: ExternalAgent,
        idle_resume: Option<ExternalAgentResume>,
        stop: CancellationToken,
    ) -> Self {
        Self {
            kernel: manager.session_kernel(),
            inputs: manager.external_agent_inputs.clone(),
            tasks: manager.control_tasks.clone(),
            session,
            scope: ExternalAgentInputScope { agent, idle_resume },
            stop,
        }
    }

    pub(super) async fn request(
        self,
        input: NativeInput,
        withdrawal: CancellationToken,
    ) -> Result<Value, AppError> {
        // Once publication has been admitted, cleanup must outlive the
        // frame reader, including a cancelled or timed-out outbound request.
        let withdraw_on_drop = withdrawal.clone().drop_guard();
        let tracker = self.tasks.clone();
        let answer = tracker
            .spawn(self.serve(input, withdrawal))
            .await
            .map_err(|error| AppError::Internal(format!("ACP input worker stopped: {error}")))?;
        withdraw_on_drop.disarm();
        answer
    }

    async fn serve(
        self,
        input: NativeInput,
        withdrawal: CancellationToken,
    ) -> Result<Value, AppError> {
        let fallback = input.reply.cancel();
        if self.stop.is_cancelled() || withdrawal.is_cancelled() {
            return Ok(fallback);
        }
        let id = RequestId::new();
        let (response, answer) = oneshot::channel();
        let (retire, retired) = oneshot::channel();
        {
            let mut inputs = self
                .inputs
                .0
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if inputs
                .keys()
                .filter(|(session, _)| *session == self.session)
                .count()
                >= MAX_EXTERNAL_AGENT_INPUTS
            {
                return Ok(fallback);
            }
            inputs.insert(
                (self.session, id),
                Pending {
                    request: UserInputRequest {
                        request_id: format!("{ID_PREFIX}{id}"),
                        session_id: Some(self.session.get()),
                        title: input.prompt.title,
                        body_markdown: input.prompt.body_markdown,
                        kind: input.prompt.kind.into(),
                        questions: input.prompt.questions,
                        auto_resolution_ms: Some(INPUT_TIMEOUT.as_millis() as u64),
                        source: UserInputSource::Host,
                        presented_at: None,
                        created_at: Utc::now(),
                    },
                    response: Some(response),
                    retired: Some(retired),
                    reply: input.reply,
                    scope: self.scope.clone(),
                    stop: self.stop.clone(),
                    withdrawal: withdrawal.clone(),
                },
            );
        }
        let command = RequestId::new();
        let publication = persist(|| {
            self.kernel.executor.set_external_agent_input(
                command,
                self.session,
                id,
                Some(self.scope.clone()),
            )
        })
        .await;
        let answer = if publication.is_ok() {
            tokio::select! {
                biased;
                _ = self.stop.cancelled() => fallback,
                _ = withdrawal.cancelled() => fallback,
                _ = tokio::time::sleep(INPUT_TIMEOUT) => fallback,
                answer = answer => answer.unwrap_or(fallback),
            }
        } else {
            fallback
        };
        self.inputs
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&(self.session, id));
        if publication.is_ok() {
            let command = RequestId::new();
            if let Err(error) = persist(|| {
                self.kernel
                    .executor
                    .set_external_agent_input(command, self.session, id, None)
            })
            .await
            {
                tracing::debug!(%error, "could not retire external agent input");
            }
        } else if let Err(error) = publication {
            tracing::debug!(%error, "could not publish external agent input");
        }
        let _ = retire.send(());
        Ok(answer)
    }
}

impl SessionManager {
    pub(in crate::session::manager) fn external_input_requests(
        &self,
        session: SessionId,
        configuration: &SessionConfiguration,
    ) -> Vec<UserInputRequest> {
        let Some(presence) = configuration
            .external_agent_inputs
            .as_ref()
            .filter(|presence| presence.epoch == self.kernel().reader.epoch())
        else {
            return Vec::new();
        };
        let inputs = self
            .external_agent_inputs
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        presence
            .ids
            .iter()
            .filter_map(|id| inputs.get(&(session, *id)))
            .filter(|input| input.is_waiting(configuration))
            .map(|input| input.request.clone())
            .collect()
    }

    async fn require_external_input_presence(
        &self,
        session: SessionId,
        id: RequestId,
    ) -> Result<ReadView<SessionConfiguration>, AppError> {
        let configuration = read_with_busy_retry(session, "external_agent_input", || {
            self.kernel().reader.configuration(session)
        })
        .await?;
        if configuration
            .external_agent_inputs
            .as_ref()
            .is_some_and(|presence| {
                presence.epoch == self.kernel().reader.epoch() && presence.ids.contains(&id)
            })
        {
            Ok(configuration)
        } else {
            Err(not_pending(id))
        }
    }

    pub(in crate::session::manager) async fn reply_external_agent_input(
        &self,
        session: SessionId,
        id: RequestId,
        reply: UserInputReply,
    ) -> Result<(), AppError> {
        let configuration = self.require_external_input_presence(session, id).await?;
        let retired = {
            let mut inputs = self
                .external_agent_inputs
                .0
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let input = inputs
                .get_mut(&(session, id))
                .filter(|input| input.is_waiting(&configuration))
                .ok_or_else(|| not_pending(id))?;
            super::super::commands::validate_answers(&input.request, &reply)?;
            let value = input.reply.answer(&reply)?;
            input
                .response
                .take()
                .expect("pending input has a responder")
                .send(value)
                .map_err(|_| not_pending(id))?;
            input
                .retired
                .take()
                .expect("pending input has a retirement receipt")
        };
        // Advance the durable session revision before acknowledging the
        // answer so a conditional read cannot replay the answered request.
        let _ = retired.await;
        Ok(())
    }

    pub(in crate::session::manager) async fn present_external_agent_input(
        &self,
        session: SessionId,
        id: RequestId,
    ) -> Result<(), AppError> {
        let configuration = self.require_external_input_presence(session, id).await?;
        let mut inputs = self
            .external_agent_inputs
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let input = inputs
            .get_mut(&(session, id))
            .filter(|input| input.is_waiting(&configuration))
            .ok_or_else(|| not_pending(id))?;
        input.request.presented_at.get_or_insert_with(Utc::now);
        Ok(())
    }
}

fn not_pending(id: RequestId) -> AppError {
    AppError::InteractiveRequestNotPending {
        request_kind: "user input".into(),
        request_id: format!("{ID_PREFIX}{id}"),
    }
}
