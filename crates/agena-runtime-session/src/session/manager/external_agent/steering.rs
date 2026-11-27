use tokio::sync::{mpsc, oneshot};

use super::*;

pub(in crate::session::manager) type Slot =
    std::sync::Mutex<Option<(ExecutionId, mpsc::Sender<Request>)>>;

pub(in crate::session::manager) struct Request {
    pub(super) text: String,
    pub(super) id: RequestId,
    pub(super) answer: oneshot::Sender<Result<(), AppError>>,
}

pub(super) struct Lease {
    dispatch: Arc<SessionDispatch>,
    execution: ExecutionId,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut slot = self
            .dispatch
            .external_steering
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if slot.as_ref().is_some_and(|(run, _)| *run == self.execution) {
            slot.take();
        }
    }
}

impl ExternalTurn {
    pub(super) fn offer_steering(&self) -> (Lease, mpsc::Receiver<Request>) {
        let dispatch = self.manager.session_dispatch(self.control.session);
        let (sender, receiver) = mpsc::channel(8);
        let execution = self.control.execution;
        *dispatch
            .external_steering
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some((execution, sender));
        (
            Lease {
                dispatch,
                execution,
            },
            receiver,
        )
    }

    pub(super) async fn record_steer(&self, request: &Request) -> Result<(), AppError> {
        let notice = serde_json::to_string(&agena_runtime_contracts::NoticePart {
            kind: "external_agent_steer".into(),
            summary: request.text.clone(),
            detail: None,
            title: Some("Additional instruction".into()),
        })?;
        persist(|| self.attempt.notice(request.id, notice.clone()))
            .await
            .map_err(kernel_error)?;
        Ok(())
    }
}

impl SessionManager {
    pub(in crate::session::manager) async fn steer_external_agent(
        &self,
        session: SessionId,
        text: String,
    ) -> Result<(), AppError> {
        if text.trim().is_empty() || text.len() > 32 * 1024 {
            return Err(AppError::Config(
                "An additional instruction must contain between 1 and 32768 bytes.".into(),
            ));
        }
        let dispatch = self.session_dispatch(session);
        let sender = dispatch
            .external_steering
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(|(_, sender)| sender.clone())
            .ok_or_else(unavailable)?;
        let (answer, response) = oneshot::channel();
        sender
            .try_send(Request {
                text,
                id: RequestId::new(),
                answer,
            })
            .map_err(|error| {
                if matches!(error, mpsc::error::TrySendError::Full(_)) {
                    AppError::Config(
                        "The external agent is already receiving additional instructions.".into(),
                    )
                } else {
                    unavailable()
                }
            })?;
        tokio::time::timeout(std::time::Duration::from_secs(30), response)
            .await
            .map_err(|_| {
                AppError::Config(
                    "The external agent did not acknowledge the additional instruction in time."
                        .into(),
                )
            })?
            .map_err(|_| unavailable())?
    }
}

fn unavailable() -> AppError {
    AppError::Config(
        "The active external agent turn does not currently accept additional instructions.".into(),
    )
}
