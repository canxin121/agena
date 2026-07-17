use super::{
    AgenaRuntime, AppError, Command, ContinueArgs, DateTime, Duration, GitPreflight, Instant,
    ModelRef, Path, Role, Session, SessionDetail, SessionRunOptions, UsageArgs, UsageStatsQuery,
    Utc,
};

pub(super) fn usage_stats_query_from_args(args: &UsageArgs) -> Result<UsageStatsQuery, AppError> {
    let has_custom_range = args.from.is_some() || args.to.is_some();
    let mut query = UsageStatsQuery::for_period_with_offset(
        args.period.into_usage_period(),
        Utc::now(),
        args.timezone_offset_minutes,
    );
    if has_custom_range {
        query = UsageStatsQuery::custom(
            args.from
                .as_deref()
                .map(|value| parse_usage_datetime(value, false))
                .transpose()?,
            args.to
                .as_deref()
                .map(|value| parse_usage_datetime(value, true))
                .transpose()?
                .or_else(|| Some(Utc::now())),
        )
        .with_timezone_offset(args.timezone_offset_minutes);
    }

    if let (Some(from), Some(to)) = (query.from.as_ref(), query.to.as_ref())
        && from > to
    {
        return Err(AppError::Config(
            "--from must be earlier than or equal to --to".to_string(),
        ));
    }

    Ok(query.with_filters(
        args.provider.clone(),
        args.model.clone(),
        args.session.clone(),
        args.include_subagents,
    ))
}

pub(super) fn parse_usage_datetime(raw: &str, end_of_day: bool) -> Result<DateTime<Utc>, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Config("usage date cannot be empty".to_string()));
    }

    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(parsed.with_timezone(&Utc));
    }

    if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let datetime = if end_of_day {
            date.and_hms_milli_opt(23, 59, 59, 999)
        } else {
            date.and_hms_milli_opt(0, 0, 0, 0)
        }
        .expect("valid date boundary");
        return Ok(datetime.and_utc());
    }

    Err(AppError::Config(format!(
        "invalid usage date `{raw}`; expected YYYY-MM-DD or RFC3339"
    )))
}

pub(super) async fn latest_event_seq(
    manager: &crate::session::SessionManager,
    session_id: i64,
) -> Result<Option<i64>, AppError> {
    Ok(manager
        .list_session_events(session_id)
        .await?
        .last()
        .map(|event| event.meta.seq_global))
}

pub(super) fn session_detail(session: &Session, latest_event_seq: Option<i64>) -> SessionDetail {
    SessionDetail {
        id: session.id,
        parent_id: session.parent_id,
        workspace_id: session.workspace_id,
        title: session.title.clone(),
        version: session.version,
        created_at: session.created_at,
        updated_at: session.updated_at,
        message_count: session.messages.len(),
        status: session.runtime.workflow.state,
        latest_event_seq,
    }
}

pub(super) fn resolve_continue_options(
    runtime: &AgenaRuntime,
    session: &Session,
    args: &ContinueArgs,
) -> Result<SessionRunOptions, AppError> {
    let snapshot = runtime.current_snapshot();
    let model = if let Some(model) = args.model.as_deref() {
        snapshot.resolve_model_target(model, None)?
    } else if let Some(model) = session
        .runtime
        .effective_model_ref()
        .map_err(|err| AppError::Config(format!("invalid persisted model reference: {err}")))?
    {
        model
    } else {
        default_model(runtime)?
    };

    let agent_profile = args
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Ok(SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: args.temperature,
        max_output_tokens: args.max_output_tokens,
        agent_profile,
    })
}

pub(super) fn resolve_run_options(
    runtime: &AgenaRuntime,
    model: Option<&str>,
    agent_profile: Option<&str>,
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
) -> Result<SessionRunOptions, AppError> {
    let model = if let Some(model) = model {
        runtime
            .current_snapshot()
            .resolve_model_target(model, None)?
    } else {
        default_model(runtime)?
    };

    let agent_profile = agent_profile
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Ok(SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature,
        max_output_tokens,
        agent_profile,
    })
}

pub(super) fn default_model(runtime: &AgenaRuntime) -> Result<ModelRef, AppError> {
    super::resolve_default_model(runtime)
}

pub(super) fn last_assistant_text(session: &Session) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .map(|message| message.visible_text_lossy())
}

pub(super) fn title_from_prompt(prompt: &str) -> String {
    let title = prompt.trim().replace('\n', " ");
    let mut chars = title.chars();
    let truncated = chars.by_ref().take(80).collect::<String>();
    if truncated.is_empty() {
        "exec".to_owned()
    } else if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

pub(super) fn git_output<const N: usize>(
    workspace_root: &Path,
    args: [&str; N],
) -> Result<String, AppError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()?;
    if !output.status.success() {
        return Err(AppError::Config(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(super) fn collect_git_preflight(workspace_root: &Path) -> Result<GitPreflight, AppError> {
    let git_available = crate::git::command_available("git");
    let gh_available = crate::git::command_available("gh");
    if !git_available {
        return Ok(GitPreflight {
            git_available,
            repo: false,
            gh_available,
            branch: None,
            upstream: None,
            ahead: None,
            behind: None,
            staged_files: 0,
            unstaged_files: 0,
            untracked_files: 0,
            changed_files: 0,
            clean: true,
        });
    }

    let repo = crate::git::succeeds(workspace_root, ["rev-parse", "--is-inside-work-tree"]);
    if !repo {
        return Ok(GitPreflight {
            git_available,
            repo,
            gh_available,
            branch: None,
            upstream: None,
            ahead: None,
            behind: None,
            staged_files: 0,
            unstaged_files: 0,
            untracked_files: 0,
            changed_files: 0,
            clean: true,
        });
    }

    let branch = non_empty_string(git_output(workspace_root, ["branch", "--show-current"])?);
    let upstream = git_output(
        workspace_root,
        [
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .ok()
    .and_then(non_empty_string);
    let (ahead, behind) = crate::git::parse_ahead_behind(
        upstream
            .as_ref()
            .and_then(|_| {
                git_output(
                    workspace_root,
                    ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
                )
                .ok()
            })
            .as_deref(),
    );
    let status = git_output(workspace_root, ["status", "--porcelain"])?;
    let status = crate::git::summarize_status(status.as_str());

    Ok(GitPreflight {
        git_available,
        repo,
        gh_available,
        branch,
        upstream,
        ahead,
        behind,
        staged_files: status.staged,
        unstaged_files: status.unstaged,
        untracked_files: status.untracked,
        changed_files: status.changed,
        clean: status.changed == 0,
    })
}

pub(super) fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub(super) fn session_storage_error() -> AppError {
    AppError::Config("session storage is unavailable; configure a database URL or path".to_owned())
}

pub(super) async fn poll_until<T, F, Fut>(
    timeout: Duration,
    interval: Duration,
    mut poll: F,
) -> Result<Option<T>, AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, AppError>>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = poll().await? {
            return Ok(Some(value));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(interval).await;
    }
}
