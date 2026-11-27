use std::collections::BTreeSet;

use agena_domain::{
    ExternalAgentArchiveFilter, ExternalAgentIndexCapabilities, ExternalAgentIndexedSession,
};

use super::lifecycle::{next_cursor, session_rows, workspace_row};
use super::*;

pub(super) const CHANGES_METHOD: &str = "_session/list/changes";
const SUBSCRIBE_METHOD: &str = "_session/list/subscribe";
const UNSUBSCRIBE_METHOD: &str = "_session/list/unsubscribe";
const BATCH_METHOD: &str = "_session/batch_get";
const ARCHIVE_METHOD: &str = "_session/archive";
const UNARCHIVE_METHOD: &str = "_session/unarchive";
pub(in crate::session::manager::external_agent) const MAX_BATCH_IDS: usize = 32;

pub(super) fn allowed_request(method: &str) -> bool {
    matches!(
        method,
        "initialize"
            | "session/list"
            | SUBSCRIBE_METHOD
            | UNSUBSCRIBE_METHOD
            | BATCH_METHOD
            | ARCHIVE_METHOD
            | UNARCHIVE_METHOD
    )
}

pub(super) struct PendingChanges {
    cwd: PathBuf,
    subscriptions: BTreeSet<String>,
}

impl PendingChanges {
    pub(super) fn observe(&mut self, params: &Value) {
        if self.subscriptions.len() < 8
            && let Some(id) = changed_subscription(params, &self.cwd)
        {
            self.subscriptions.insert(id);
        }
    }
}

fn changed_subscription(params: &Value, cwd: &Path) -> Option<String> {
    let id = bounded_string(&params["subscriptionId"], 512)?;
    let updated = params["sessions"].as_array().is_some_and(|rows| {
        rows.iter()
            .take(1_000)
            .any(|row| workspace_row(row, cwd).is_some())
    });
    let removed = params["removed"].as_array().is_some_and(|ids| {
        ids.iter()
            .take(1_000)
            .any(|id| bounded_string(id, 4096).is_some())
    });
    // A removal contains an ID without a path. Treat it as a signal to
    // refresh the scoped page and keep the native ID inside this connection.
    (updated || removed).then_some(id)
}

fn index_row(row: &Value, cwd: &Path) -> Option<ExternalAgentIndexedSession> {
    let session = workspace_row(row, cwd)?;
    let air = &row["_meta"]["jetbrains"]["air"];
    Some(ExternalAgentIndexedSession {
        session,
        archived: air["archived"].as_bool(),
        state: bounded_string(&air["state"], 64),
        model: bounded_string(&air["model"], 512),
        last_prompt_at: bounded_string(&air["lastPromptAt"], 128),
    })
}

impl AcpClient {
    pub(in crate::session::manager::external_agent) async fn open_index(
        spec: ResolvedExternalAgent,
        cwd: &Path,
        cancel: &CancellationToken,
    ) -> Result<Option<(Self, ExternalAgentIndexCapabilities)>, AppError> {
        if spec.id != ExternalAgent::ClaudeCode {
            return Ok(None);
        }
        let mut client = Self::spawn(spec, cwd)?;
        client.air_index = true;
        let initialized = client.initialization(cancel).await;
        let result = initialized.map(|response| {
            let supports = |capability| protocol::advertises_air_capability(response, capability);
            let index = supports("sessionIndex");
            let batch_get = index && supports("sessionBatchGet");
            ExternalAgentIndexCapabilities {
                index,
                archive: batch_get && supports("sessionArchive"),
                batch_get,
                watch: index && supports("sessionListSubscribe"),
            }
        });
        match result {
            Ok(capabilities) if capabilities.index => Ok(Some((client, capabilities))),
            result => {
                client.close().await;
                result.map(|_| None)
            }
        }
    }

    pub(in crate::session::manager::external_agent) async fn list_indexed(
        &mut self,
        cwd: &Path,
        cursor: Option<&str>,
        archived: ExternalAgentArchiveFilter,
        cancel: &CancellationToken,
    ) -> Result<(Vec<ExternalAgentIndexedSession>, Option<String>), AppError> {
        let mut params = json!({
            "cwd":cwd,
            "_meta":{"jetbrains":{"air":{"version":1, "list":{
                "archived":archived, "includeWorktrees":false
            }}}},
        });
        if let Some(cursor) = cursor {
            params["cursor"] = json!(cursor);
        }
        let response = self.call("session/list", params, cancel).await?;
        let sessions = session_rows(&response)?
            .iter()
            .filter_map(|row| index_row(row, cwd))
            .filter(|row| match archived {
                ExternalAgentArchiveFilter::Unarchived => row.archived != Some(true),
                ExternalAgentArchiveFilter::Archived => row.archived == Some(true),
                ExternalAgentArchiveFilter::All => true,
            })
            .collect();
        Ok((sessions, next_cursor(&response)))
    }

    pub(in crate::session::manager::external_agent) async fn batch_indexed(
        &mut self,
        cwd: &Path,
        ids: &[String],
        cancel: &CancellationToken,
    ) -> Result<Vec<ExternalAgentIndexedSession>, AppError> {
        let response = self
            .call(BATCH_METHOD, json!({"sessionIds": ids}), cancel)
            .await?;
        let mut seen = BTreeSet::new();
        Ok(session_rows(&response)?
            .iter()
            .filter_map(|row| index_row(row, cwd))
            .filter(|row| {
                ids.contains(&row.session.native_id) && seen.insert(row.session.native_id.clone())
            })
            .collect())
    }

    pub(in crate::session::manager::external_agent) async fn set_archived_indexed(
        &mut self,
        cwd: &Path,
        native_id: String,
        archived: bool,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        let ids = [native_id];
        let Some(row) = self
            .batch_indexed(cwd, &ids, cancel)
            .await?
            .into_iter()
            .next()
        else {
            return Err(AppError::Config(
                "That native conversation is not listed in this workspace.".into(),
            ));
        };
        if row.archived == Some(archived) {
            return Ok(());
        }
        if matches!(row.state.as_deref(), Some("running" | "requires_action")) {
            return Err(AppError::Config(
                "Finish or stop the native turn before changing its archive state.".into(),
            ));
        }
        let method = if archived {
            ARCHIVE_METHOD
        } else {
            UNARCHIVE_METHOD
        };
        self.call(method, json!({"sessionId": ids[0]}), cancel)
            .await?;
        Ok(())
    }

    pub(in crate::session::manager::external_agent) async fn watch_indexed(
        &mut self,
        cwd: &Path,
        wait: Duration,
        cancel: &CancellationToken,
    ) -> Result<bool, AppError> {
        let mut early = PendingChanges {
            cwd: cwd.to_owned(),
            subscriptions: BTreeSet::new(),
        };
        let response = self
            .request_for(
                STARTUP_TIMEOUT,
                SUBSCRIBE_METHOD,
                json!({"cwd":cwd}),
                SessionUpdates::IndexChanges(&mut early),
                cancel,
            )
            .await
            .map_err(|_| {
                AppError::Config(
                    "Claude Code did not confirm its session index subscription.".into(),
                )
            })??;
        let subscription = bounded_string(&response["subscriptionId"], 512).ok_or_else(|| {
            AppError::Internal(
                "Claude Code returned an invalid session index subscription ID.".into(),
            )
        })?;
        let result = if early.subscriptions.contains(&subscription) {
            Ok(true)
        } else {
            tokio::time::timeout(wait, self.read_index_changes(cwd, &subscription, cancel))
                .await
                .unwrap_or(Ok(false))
        };
        // The caller may have disconnected. Give the peer a short cleanup
        // window with its own token, then the outer owner closes the process.
        let _ = self
            .request_for(
                Duration::from_secs(2),
                UNSUBSCRIBE_METHOD,
                json!({"subscriptionId":subscription}),
                SessionUpdates::Ignore,
                &CancellationToken::new(),
            )
            .await;
        result
    }

    async fn read_index_changes(
        &mut self,
        cwd: &Path,
        subscription: &str,
        cancel: &CancellationToken,
    ) -> Result<bool, AppError> {
        loop {
            let line = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                line = self.output.next() => line,
            };
            let Some(line) = line else {
                return Err(AppError::Config(
                    "Claude Code closed its session index subscription.".into(),
                ));
            };
            let line = line
                .map_err(|error| AppError::Internal(format!("invalid ACP index frame: {error}")))?;
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(&line)?;
            if message["method"] == CHANGES_METHOD
                && message.get("id").is_none()
                && changed_subscription(&message["params"], cwd).as_deref() == Some(subscription)
            {
                return Ok(true);
            }
            if message.get("method").is_some()
                && let Some(id) = message.get("id")
            {
                self.write(
                    &Reply::Error(-32601, "Unsupported ACP index client request").message(id),
                    cancel,
                )
                .await?;
            }
        }
    }
}
