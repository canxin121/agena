use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use agena_domain::{
    ExternalAgentArchiveFilter, ExternalAgentIndexedSession, ExternalAgentSessionCapabilities,
    ExternalAgentSessionInfo,
};
use agena_storage::kernel::ExternalAgentHistoryTurn;

use super::*;

mod replay;
pub(super) use replay::Replay;

#[derive(Clone, Copy, Default)]
pub(in crate::session::manager::external_agent) struct LifecycleCapabilities {
    pub(super) list: bool,
    archive: bool,
    pub(super) additional_directories: bool,
    pub(super) system_prompt: bool,
    fork: bool,
    status: bool,
    title: bool,
}

impl LifecycleCapabilities {
    pub(super) fn read(session: &Value, v2: bool) -> Self {
        let supports = |key: &str| session[key].is_object();
        Self {
            list: supports("list"),
            archive: supports("archive") && (v2 || supports("list")),
            additional_directories: supports("additionalDirectories"),
            system_prompt: supports("systemPrompt"),
            fork: supports("fork"),
            status: supports("status"),
            title: supports("title"),
        }
    }
}

#[derive(Default)]
pub(in crate::session::manager::external_agent) struct SessionContext {
    pub additional_directories: Vec<PathBuf>,
    pub system_prompt: String,
}

impl SessionContext {
    pub(super) fn apply(
        &self,
        params: &mut Value,
        caps: LifecycleCapabilities,
    ) -> Result<(), AppError> {
        if !self.additional_directories.is_empty() {
            if !caps.additional_directories {
                return Err(AppError::Config(
                    "The installed agent does not support additional workspace directories over ACP.".into(),
                ));
            }
            params["additionalDirectories"] = json!(self.additional_directories);
        }
        if caps.system_prompt && !self.system_prompt.trim().is_empty() {
            params["systemPrompt"] = json!({
                "mode": "append",
                "content": [{"type": "text", "text": self.system_prompt}],
            });
        }
        Ok(())
    }
}

pub(in crate::session::manager::external_agent) struct NativePage {
    pub sessions: Vec<ExternalAgentIndexedSession>,
    pub next_cursor: Option<String>,
}

impl AcpClient {
    pub(in crate::session::manager::external_agent) async fn lifecycle_capabilities(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<ExternalAgentSessionCapabilities, AppError> {
        let caps = self.initialize(cancel).await?;
        Ok(ExternalAgentSessionCapabilities {
            status: caps.lifecycle.status,
            title: caps.lifecycle.title,
            close: caps.close,
            delete: caps.delete,
            fork: caps.lifecycle.fork,
            fork_before_message: caps.lifecycle.fork && self.uses_v2(),
            additional_directories: caps.lifecycle.additional_directories,
            system_prompt: caps.lifecycle.system_prompt,
        })
    }

    pub(in crate::session::manager::external_agent) async fn list_native(
        &mut self,
        cwd: &Path,
        cursor: Option<&str>,
        filter: ExternalAgentArchiveFilter,
        cancel: &CancellationToken,
    ) -> Result<NativePage, AppError> {
        let caps = self.initialize(cancel).await?.lifecycle;
        if !caps.list && !self.uses_v2() {
            return Err(self.unsupported_lifecycle("listing conversations"));
        }
        let mut params = json!({"cwd": cwd});
        if caps.archive {
            params["archived"] = json!(match filter {
                ExternalAgentArchiveFilter::Unarchived => "exclude",
                ExternalAgentArchiveFilter::Archived => "only",
                ExternalAgentArchiveFilter::All => "include",
            });
        } else if filter != ExternalAgentArchiveFilter::Unarchived {
            return Err(self.unsupported_lifecycle("listing archived conversations"));
        }
        if let Some(cursor) = cursor {
            params["cursor"] = json!(cursor);
        }
        let response = self
            .lifecycle_request("session/list", params, SessionUpdates::Ignore, cancel)
            .await?;
        let mut sessions = Vec::new();
        for row in session_rows(&response)? {
            let Some(session) = workspace_row(row, cwd) else {
                continue;
            };
            let mut listed = ExternalAgentIndexedSession::from(session);
            if caps.archive {
                let archived = row["archived"].as_bool().ok_or_else(|| {
                    AppError::Internal(
                        "The external agent omitted the archive state of a listed conversation."
                            .into(),
                    )
                })?;
                if matches!(filter, ExternalAgentArchiveFilter::Archived) && !archived
                    || matches!(filter, ExternalAgentArchiveFilter::Unarchived) && archived
                {
                    continue;
                }
                listed.archived = Some(archived);
            }
            sessions.push(listed);
        }
        Ok(NativePage {
            sessions,
            next_cursor: next_cursor(&response),
        })
    }

    pub(in crate::session::manager::external_agent) async fn supports_native_archive(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<bool, AppError> {
        Ok(self.initialize(cancel).await?.lifecycle.archive)
    }

    pub(in crate::session::manager::external_agent) async fn find_native(
        &mut self,
        cwd: &Path,
        native_id: &str,
        filter: ExternalAgentArchiveFilter,
        cancel: &CancellationToken,
    ) -> Result<Option<ExternalAgentIndexedSession>, AppError> {
        let mut cursor = None;
        let mut seen = BTreeSet::new();
        loop {
            let page = self
                .list_native(cwd, cursor.as_deref(), filter, cancel)
                .await?;
            if let Some(row) = page
                .sessions
                .into_iter()
                .find(|row| row.session.native_id == native_id)
            {
                return Ok(Some(row));
            }
            let Some(next) = page.next_cursor else {
                return Ok(None);
            };
            if seen.len() >= 99 || !seen.insert(next.clone()) {
                return Err(AppError::Config(
                    "Could not find the native conversation within the session listing budget."
                        .into(),
                ));
            }
            cursor = Some(next);
        }
    }

    pub(in crate::session::manager::external_agent) async fn set_native_archived(
        &mut self,
        cwd: &Path,
        native_id: &str,
        archived: bool,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        if !self.supports_native_archive(cancel).await? {
            return Err(self.unsupported_lifecycle("changing the archive state of conversations"));
        }
        let row = self
            .find_native(cwd, native_id, ExternalAgentArchiveFilter::All, cancel)
            .await?
            .ok_or_else(|| {
                AppError::Config("That native conversation is not listed in this workspace.".into())
            })?;
        if row.archived == Some(archived) {
            return Ok(());
        }
        let method = if archived {
            "session/archive"
        } else {
            "session/unarchive"
        };
        let response = self
            .lifecycle_request(
                method,
                json!({"sessionId": native_id}),
                SessionUpdates::Ignore,
                cancel,
            )
            .await?;
        if !response.is_object() {
            return Err(AppError::Internal(
                "The external agent returned an invalid archive response.".into(),
            ));
        }
        Ok(())
    }

    pub(in crate::session::manager::external_agent) async fn replay_native(
        &mut self,
        cwd: &Path,
        native_id: &str,
        context: &SessionContext,
        cancel: &CancellationToken,
    ) -> Result<(Vec<ExternalAgentHistoryTurn>, ExternalAgentState), AppError> {
        let capabilities = self.initialize(cancel).await?;
        let mut params = json!({"cwd": cwd, "sessionId": native_id, "mcpServers": []});
        context.apply(&mut params, capabilities.lifecycle)?;
        let method = if self.uses_v2() {
            params["replayFrom"] = json!({"type": "start"});
            "session/resume"
        } else if capabilities.load {
            "session/load"
        } else {
            return Err(self.unsupported_lifecycle("reading conversation history"));
        };
        self.prepare_provider_routes(cancel).await?;
        let mut replay = Replay::default();
        let response = self
            .lifecycle_request(
                method,
                params,
                SessionUpdates::Replay(&mut replay, native_id),
                cancel,
            )
            .await?;
        if self.uses_v2() && !replay.complete {
            tracing::debug!("ACP v2 resume succeeded without a replay_complete update");
        }
        replay.apply_setup_response(&response, self.uses_v2());
        replay.finish(self.agent, cwd).await
    }

    pub(in crate::session::manager::external_agent) async fn fork_native(
        &mut self,
        cwd: &Path,
        native_id: &str,
        cursor: Option<&str>,
        context: &SessionContext,
        cancel: &CancellationToken,
    ) -> Result<String, AppError> {
        let caps = self.initialize(cancel).await?.lifecycle;
        if !caps.fork {
            return Err(self.unsupported_lifecycle("forking conversations"));
        }
        let mut params = json!({"cwd": cwd, "sessionId": native_id, "mcpServers": []});
        if let Some(cursor) = cursor {
            if !self.uses_v2() {
                return Err(self.unsupported_lifecycle("forking before an earlier message"));
            }
            params["cursorId"] = json!(cursor);
        }
        context.apply(&mut params, caps)?;
        self.prepare_provider_routes(cancel).await?;
        let response = self
            .lifecycle_request("session/fork", params, SessionUpdates::Ignore, cancel)
            .await?;
        bounded_string(&response["sessionId"], 4096)
            .filter(|id| id != native_id)
            .ok_or_else(|| {
                AppError::Internal("The external agent returned an invalid fork ID.".into())
            })
    }

    pub(in crate::session::manager::external_agent) async fn native_status(
        &mut self,
        native_id: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, AppError> {
        if !self.initialize(cancel).await?.lifecycle.status {
            return Ok(None);
        }
        let response = self
            .lifecycle_request(
                "session/status",
                json!({"sessionId": native_id}),
                SessionUpdates::Ignore,
                cancel,
            )
            .await?;
        Ok(bounded_string(&response["status"], 128))
    }

    pub(in crate::session::manager::external_agent) async fn set_native_title(
        &mut self,
        native_id: &str,
        title: &str,
        cancel: &CancellationToken,
    ) -> Result<String, AppError> {
        if !self.initialize(cancel).await?.lifecycle.title {
            return Err(self.unsupported_lifecycle("renaming conversations"));
        }
        let response = self
            .lifecycle_request(
                "session/set_title",
                json!({"sessionId": native_id, "title": title}),
                SessionUpdates::Ignore,
                cancel,
            )
            .await?;
        bounded_title(&response["title"]).ok_or_else(|| {
            AppError::Internal(
                "The external agent did not return the saved conversation title.".into(),
            )
        })
    }

    pub(in crate::session::manager::external_agent) async fn release_native(
        &mut self,
        native_id: &str,
        delete: bool,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        let capabilities = self.initialize(cancel).await?;
        if (delete && !capabilities.delete) || (!delete && !capabilities.close) {
            return Err(self.unsupported_lifecycle(if delete {
                "deleting conversations"
            } else {
                "closing conversations"
            }));
        }
        let method = if delete {
            "session/delete"
        } else {
            "session/close"
        };
        self.lifecycle_request(
            method,
            json!({"sessionId": native_id}),
            SessionUpdates::Ignore,
            cancel,
        )
        .await?;
        Ok(())
    }

    async fn lifecycle_request(
        &mut self,
        method: &str,
        params: Value,
        updates: SessionUpdates<'_>,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        let agent = self.agent;
        self.request_for(STARTUP_TIMEOUT, method, params, updates, cancel)
            .await
            .map_err(|_| {
                AppError::Config(format!("{} timed out during {method}.", agent.label()))
            })?
    }

    fn unsupported_lifecycle(&self, operation: &str) -> AppError {
        AppError::Config(format!(
            "{} does not support {operation} over ACP.",
            self.agent.label()
        ))
    }
}

pub(super) fn session_rows(response: &Value) -> Result<&[Value], AppError> {
    let rows = response["sessions"].as_array().ok_or_else(|| {
        AppError::Internal("The external agent returned an invalid session list.".into())
    })?;
    if rows.len() > 1_000 {
        return Err(AppError::Internal(
            "The external agent returned an oversized session list.".into(),
        ));
    }
    Ok(rows)
}

pub(super) fn workspace_row(row: &Value, cwd: &Path) -> Option<ExternalAgentSessionInfo> {
    let path = PathBuf::from(bounded_string(&row["cwd"], 8192)?);
    // Never allow a peer to redirect a native operation to its claimed CWD.
    if path != cwd {
        return None;
    }
    Some(ExternalAgentSessionInfo {
        native_id: bounded_string(&row["sessionId"], 4096)?,
        cwd: path,
        title: bounded_title(&row["title"]),
        updated_at: bounded_string(&row["updatedAt"], 128),
    })
}

pub(super) fn next_cursor(response: &Value) -> Option<String> {
    response["nextCursor"]
        .as_str()
        .filter(|cursor| cursor.len() <= MAX_CURSOR_BYTES)
        .map(str::to_owned)
}

pub(in crate::session::manager::external_agent) fn bounded_title(value: &Value) -> Option<String> {
    let title = value.as_str()?.trim();
    if title.is_empty() || title.chars().any(char::is_control) {
        return None;
    }
    Some(title[..title.floor_char_boundary(4096)].to_owned())
}

pub(in crate::session::manager::external_agent) fn bounded_string(
    value: &Value,
    max: usize,
) -> Option<String> {
    let value = value.as_str()?;
    (!value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}
