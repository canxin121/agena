use agena_domain::ProcessStatus;
use agena_plugin_host::PluginError;
use agena_plugin_sdk::ImportedClaudeStopWork;
use agena_scheduler::JobKind;
use agena_storage::kernel::{BackgroundActivityKind, SessionId};
use serde_json::json;

use super::{RuntimeHostClient, is_transcript_parent, plugin_error_from_app};

const WORK_LIMIT: usize = 128;

impl RuntimeHostClient {
    pub(super) async fn claude_stop_work(
        &self,
        parent: bool,
        callback_generation: Option<u64>,
    ) -> Result<Option<ImportedClaudeStopWork>, PluginError> {
        let (id, call) = self.callback_scope()?;
        let plugin = self.imported_agent_plugin()?;
        if callback_generation != Some(plugin.effect_scope().generation())
            || !plugin
                .agent_plugin()
                .is_some_and(|package| package.has_claude_stop_hooks())
            || id <= 0
        {
            return Ok(None);
        }
        let mut session =
            SessionId::try_from(id).map_err(|error| PluginError::invalid_params_error(&error))?;
        let manager = self.session_manager()?;
        let reader = &manager.kernel().reader;
        let key = plugin.key();
        let mut executor = manager
            .host_resource_callback_executor(Some(id), (call >= 0).then_some(call))
            .await
            .map_err(plugin_error_from_app)?;
        if executor.is_global() || !executor.plugin_is_enabled(&key) {
            return Ok(None);
        }
        if parent {
            let child = reader
                .summary(session)
                .await
                .map_err(|error| PluginError::internal_error(&error))?;
            let Some(parent) = child
                .parent_session_id
                .filter(|_| child.relation_kind.is_subagent())
            else {
                return Ok(None);
            };
            let summary = reader
                .summary(parent)
                .await
                .map_err(|error| PluginError::internal_error(&error))?;
            if !is_transcript_parent(&child, &summary) {
                return Ok(None);
            }
            executor = manager
                .host_resource_callback_executor(Some(parent.get()), None)
                .await
                .map_err(plugin_error_from_app)?;
            if executor.is_global() || !executor.plugin_is_enabled(&key) {
                return Ok(None);
            }
            session = parent;
        }

        let (activities, schedules) =
            tokio::join!(reader.active_activities(session, None, WORK_LIMIT), async {
                match executor.scheduler() {
                    Some(scheduler) => scheduler.pending_jobs_for_session(session.get()).await,
                    None => Ok(Vec::new()),
                }
            },);
        let activities = activities.map_err(|error| PluginError::internal_error(&error))?;
        let schedules = schedules.map_err(|error| PluginError::internal_error(&error))?;
        let managed_tasks = activities.activities.iter().filter_map(|activity| {
            let kind = match activity.start.kind {
                BackgroundActivityKind::Agent => "subagent",
                BackgroundActivityKind::Monitor => "monitor",
                BackgroundActivityKind::Process | BackgroundActivityKind::Terminal => {
                    return None;
                }
            };
            Some(json!({
                "id": activity.id.to_string(),
                "type": kind,
                "status": "running",
                "description": clipped(&activity.start.label),
            }))
        });
        let processes = executor
            .monitor_registry()
            .map_or_else(Vec::new, |registry| registry.list_session(session.get()));
        let shell_tasks = processes.into_iter()
            .filter(|process| process.status == ProcessStatus::Running && process.background)
            .map(|process| json!({
                "id": process.process_id,
                "type": if process.monitored || process.websocket { "monitor" } else { "shell" },
                "status": "running",
                "description": clipped(&process.description),
                "command": clipped(&process.command),
            }));
        let background_tasks = managed_tasks.chain(shell_tasks).take(WORK_LIMIT).collect();
        let session_crons = schedules
            .into_iter()
            .take(WORK_LIMIT)
            .map(|job| {
                let (schedule, recurring) = match job.kind {
                    JobKind::Cron { expression, .. } => (expression, true),
                    JobKind::Once { at } => (at.format("%S %M %H %d %m * %Y").to_string(), false),
                };
                json!({
                    "id": job.id.to_string(),
                    "schedule": schedule,
                    "recurring": recurring,
                    "prompt": clipped(&job.prompt),
                })
            })
            .collect();
        Ok(Some(ImportedClaudeStopWork {
            background_tasks,
            session_crons,
        }))
    }
}

fn clipped(text: &str) -> String {
    const LIMIT: usize = 1000;
    let count = text.chars().count();
    if count <= LIMIT {
        return text.to_owned();
    }
    let preview_chars = LIMIT - 32;
    let end = text
        .char_indices()
        .nth(preview_chars)
        .expect("text exceeds limit")
        .0;
    format!("{}… [+{} chars]", &text[..end], count - preview_chars)
}
