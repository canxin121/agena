//! Bot identities in their own table, and ownership projected onto sessions.
//!
//! A Bot has no single bound conversation. Its Web/TUI conversation and its
//! chat conversations are ordinary sessions whose configuration carries
//! `owner.bot`; the kernel projects that into indexed `owner_*` columns. This
//! module owns the Bot identity table, the ownership-based admission checks and
//! the lookup that every conversation shares.
use super::*;
use agena_assistant::bot::*;
use agena_domain::{SessionOwner, SessionSurface};
use agena_storage::{BotRecord, BotRepositoryError};
use caseless::Caseless;
use uuid::Uuid;

const MAX_BOTS: u64 = 256;

fn conflict(message: &'static str) -> AppError {
    AppError::BotConflict(message)
}

fn validate_key(key: &str) -> Result<(), AppError> {
    if key.is_empty() || key.len() > 160 || key.trim() != key || key.chars().any(char::is_control) {
        return Err(invalid(
            "request_key must contain 1..160 non-control bytes without surrounding whitespace",
        ));
    }
    Ok(())
}

/// The uniqueness key for active Bot names: trimmed and case-folded.
pub(super) fn folded_name(name: &str) -> String {
    name.trim().chars().default_case_fold().collect()
}

fn validate_name(name: &str, persona: &BotPersona) -> Result<String, AppError> {
    if name.chars().any(char::is_control) {
        return Err(invalid("Bot names cannot contain control characters"));
    }
    let name = name.trim();
    persona
        .validate(name)
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(name.into())
}

fn repository_error(error: BotRepositoryError) -> AppError {
    match error {
        BotRepositoryError::Invalid(message) => invalid(&message),
        BotRepositoryError::NameTaken => conflict("an active Bot already has this name"),
        BotRepositoryError::Conflict => {
            conflict("stale Bot revision; reload the profile before editing")
        }
        BotRepositoryError::NotFound => {
            AppError::BotNotFound("Bot was not found; select an existing Bot or create one".into())
        }
        BotRepositoryError::Capacity => {
            invalid("Bot registry limit is 256; archived histories also count")
        }
        BotRepositoryError::ReplayedInput => {
            conflict("request_key belongs to a different Bot operation payload")
        }
        BotRepositoryError::Backend(message) => AppError::Internal(message),
    }
}

fn profile(record: BotRecord) -> BotProfile {
    BotProfile {
        id: record.id,
        name: record.name,
        persona: BotPersona {
            instructions: record.instructions,
        },
        revision: record.revision,
        archived: record.archived,
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
    }
}

fn record(profile: &BotProfile) -> BotRecord {
    BotRecord {
        id: profile.id,
        name: profile.name.clone(),
        name_folded: folded_name(&profile.name),
        instructions: profile.persona.instructions.clone(),
        revision: profile.revision,
        archived: profile.archived,
        created_at_ms: profile.created_at_ms,
        updated_at_ms: profile.updated_at_ms,
    }
}

impl SessionManager {
    fn bots(&self) -> &dyn agena_storage::BotRepository {
        self.kernel_runtime.bots.as_ref()
    }

    /// Short instance-wide gate for Bot creation and first-open.
    ///
    /// The identity row is protected by its own unique index, but "does this
    /// Bot already have a local conversation?" and "create one" must be one
    /// step: without the gate, concurrent first opens would each create a
    /// conversation. The gate covers only that sequence — reads, admission and
    /// every message path stay lock-free.
    fn bot_creation_gate(&self) -> Arc<tokio::sync::Mutex<()>> {
        card_lock(format!(
            "bot-creation:{}",
            self.kernel_runtime.instance_state.path().display()
        ))
    }

    /// Ownership recorded on one conversation, read from its configuration.
    pub(super) async fn session_owner(&self, id: i64) -> Result<Option<SessionOwner>, AppError> {
        Ok(self
            .kernel()
            .reader
            .configuration(session_id(id)?)
            .await
            .map_err(kernel_error)?
            .owner
            .clone())
    }

    /// Create a conversation owned by `bot` and return its id.
    ///
    /// `Local` conversations run in the instance scope with no working
    /// directory; chat conversations are created by the channel path, which
    /// already knows its isolated workspace and authority.
    async fn create_bot_conversation(
        &self,
        bot: Uuid,
        surface: SessionSurface,
        title: &str,
    ) -> Result<i64, AppError> {
        let owner = SessionOwner {
            bot,
            surface,
            scope_key: None,
        };
        // A fresh key per creation. The caller always looks for an existing
        // local root first, so a retry after a lost reply reuses that root
        // instead of reaching this path again; a deleted root must produce a
        // genuinely new conversation, not replay the deleted one's receipt.
        let command = request_key(
            "bot-conversation",
            &format!("{bot}:{}:{}", surface.as_str(), Uuid::new_v4()),
        );
        let configuration = SessionConfiguration {
            owner: Some(owner),
            ..Default::default()
        };
        persist(|| {
            self.kernel()
                .sessions
                .create_global(command, title.to_owned(), configuration.clone())
        })
        .await
        .map(|receipt| receipt.value.id.get())
        .map_err(kernel_error)
    }

    /// The Bot's most recently used local root, if it still exists.
    pub(super) async fn default_bot_conversation(
        &self,
        bot: Uuid,
    ) -> Result<Option<i64>, AppError> {
        let page = self
            .kernel()
            .reader
            .query_summaries(SummaryQuery {
                owner_bot_id: Some(bot.to_string()),
                owner_surface: Some(SessionSurface::Local.as_str().to_owned()),
                roots_only: true,
                include_global: true,
                limit: 1,
                ..Default::default()
            })
            .await
            .map_err(kernel_error)?;
        Ok(page.sessions.first().map(|summary| summary.id.get()))
    }

    /// Replay a receipt for `key`, or refuse a reused key with new input.
    async fn replay(
        &self,
        key: &str,
        input: &impl Serialize,
    ) -> Result<Option<BotProfile>, AppError> {
        let input = serde_json::to_string(input)?;
        match self.bots().receipt(key).await.map_err(repository_error)? {
            Some((stored, _)) if stored != input => {
                Err(repository_error(BotRepositoryError::ReplayedInput))
            }
            Some((_, result)) => Ok(Some(profile(
                serde_json::from_str(&result).map_err(AppError::from)?,
            ))),
            None => Ok(None),
        }
    }

    async fn remember(
        &self,
        key: &str,
        input: &impl Serialize,
        result: &BotProfile,
    ) -> Result<(), AppError> {
        let input = serde_json::to_string(input)?;
        let result = serde_json::to_string(&record(result))?;
        self.bots()
            .save_receipt(key, &input, &result)
            .await
            .map_err(repository_error)
    }

    /// The first active Bot, or `None` when every Bot is archived.
    async fn default_bot(&self) -> Result<Option<BotProfile>, AppError> {
        Ok(self
            .bots()
            .list(false)
            .await
            .map_err(repository_error)?
            .into_iter()
            .next()
            .map(profile))
    }

    async fn bot_or_not_found(&self, id: Uuid) -> Result<BotProfile, AppError> {
        self.bots()
            .get(id)
            .await
            .map(profile)
            .map_err(repository_error)
    }

    /// Archive-state check shared by every write into a Bot conversation.
    /// `None` when `source` belongs to no Bot at all.
    pub(super) async fn bot_owner_of(
        &self,
        source: i64,
    ) -> Result<Option<(BotProfile, SessionOwner)>, AppError> {
        let Some(owner) = self.session_owner(source).await? else {
            return Ok(None);
        };
        let profile = self.bot_or_not_found(owner.bot).await?;
        Ok(Some((profile, owner)))
    }

    pub async fn list_bot_profiles(&self, include_archived: bool) -> Result<BotList, AppError> {
        let records = self
            .bots()
            .list(include_archived)
            .await
            .map_err(repository_error)?;
        let items = records.into_iter().map(profile).collect::<Vec<_>>();
        let default_bot_id = self.default_bot().await?.map(|bot| bot.id);
        Ok(BotList {
            items,
            default_bot_id,
        })
    }

    pub async fn get_bot_profile(&self, id: Uuid) -> Result<BotProfile, AppError> {
        self.bot_or_not_found(id).await
    }

    pub async fn create_bot_profile(&self, create: BotCreate) -> Result<BotProfile, AppError> {
        validate_key(&create.request_key)?;
        let persona = create.persona.clone().unwrap_or_default();
        validate_name(&create.name, &persona)?;
        let key = format!("create:{}", create.request_key);
        // Creation is the one place where "is this request already done?" and
        // "insert it" must be one step: the unique name index makes concurrent
        // identical requests fail as name conflicts unless the loser can see
        // the winner's receipt. The gate covers only this short sequence.
        let _gate = self.bot_creation_gate().lock_owned().await;
        if let Some(profile) = self.replay(&key, &create).await? {
            return Ok(profile);
        }
        if self.bots().count().await.map_err(repository_error)? >= MAX_BOTS {
            return Err(repository_error(BotRepositoryError::Capacity));
        }
        let name = validate_name(&create.name, &persona)?;
        let now = chrono::Utc::now().timestamp_millis();
        let candidate = BotProfile {
            id: Uuid::new_v4(),
            name,
            persona,
            revision: 1,
            archived: false,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let created = self
            .bots()
            .create(&record(&candidate))
            .await
            .map_err(repository_error)?;
        let created = profile(created);
        // The Bot's first conversation is an ordinary local root owned by it.
        // Created after the identity row so a crash leaves an identity with no
        // conversation, which `ensure_named_bot_session` repairs on open.
        self.create_bot_conversation(created.id, SessionSurface::Local, &created.name)
            .await?;
        self.remember(&key, &create, &created).await?;
        Ok(created)
    }

    pub async fn update_bot_profile(
        &self,
        id: Uuid,
        update: BotUpdate,
    ) -> Result<BotProfile, AppError> {
        validate_key(&update.request_key)?;
        let key = format!("update:{id}:{}", update.request_key);
        if let Some(profile) = self.replay(&key, &update).await? {
            return Ok(profile);
        }
        let current = self.bot_or_not_found(id).await?;
        if current.archived {
            return Err(conflict("archived Bot history is read-only"));
        }
        if current.revision != update.expected_revision {
            return Err(repository_error(BotRepositoryError::Conflict));
        }
        let name = validate_name(&update.name, &update.persona)?;
        let persona = update.persona.clone();
        let next = BotProfile {
            name,
            persona,
            revision: update.expected_revision + 1,
            updated_at_ms: chrono::Utc::now().timestamp_millis(),
            ..current
        };
        let updated = self
            .bots()
            .update(&record(&next), update.expected_revision)
            .await
            .map_err(repository_error)?;
        let updated = profile(updated);
        self.remember(&key, &update, &updated).await?;
        Ok(updated)
    }

    pub async fn archive_bot_profile(
        &self,
        id: Uuid,
        archive: BotArchive,
    ) -> Result<BotProfile, AppError> {
        validate_key(&archive.request_key)?;
        let key = format!("archive:{id}:{}", archive.request_key);
        if let Some(profile) = self.replay(&key, &archive).await? {
            return Ok(profile);
        }
        let current = self.bot_or_not_found(id).await?;
        if current.archived {
            return Err(conflict("stale or archived Bot; reload the profile"));
        }
        if current.revision != archive.expected_revision {
            return Err(conflict("stale or archived Bot; reload the profile"));
        }
        self.require_bot_idle(id).await?;
        let now = chrono::Utc::now().timestamp_millis();
        let archived = self
            .bots()
            .archive(id, archive.expected_revision, now)
            .await
            .map_err(repository_error)?;
        let archived = profile(archived);
        self.remember(&key, &archive, &archived).await?;
        Ok(archived)
    }

    /// Refuse to archive while any conversation of this Bot still holds work.
    ///
    /// Chat conversations are checked too: a channel transcript that can still
    /// run would otherwise keep writing into an archived Bot.
    async fn require_bot_idle(&self, id: Uuid) -> Result<(), AppError> {
        let mut before = None;
        loop {
            let page = self
                .kernel()
                .reader
                .query_summaries(SummaryQuery {
                    owner_bot_id: Some(id.to_string()),
                    roots_only: false,
                    include_global: true,
                    before,
                    limit: 128,
                    ..Default::default()
                })
                .await
                .map_err(kernel_error)?;
            for summary in &page.sessions {
                if summary
                    .agent_status
                    .is_some_and(|status| !status.is_terminal())
                    || summary.queued_executions > 0
                    || summary.running_executions > 0
                {
                    return Err(conflict(
                        "Bot must be idle with no queued executions or pending native interactions before archiving",
                    ));
                }
                let control = self
                    .kernel()
                    .reader
                    .control_snapshot(summary.id, 1)
                    .await
                    .map_err(kernel_error)?;
                if !control.pending_interactions.interactions.is_empty()
                    || !control.active_executions.executions.is_empty()
                {
                    return Err(conflict(
                        "Bot has a pending native interaction; finish it before archiving",
                    ));
                }
                if !self
                    .kernel()
                    .reader
                    .active_activities(summary.id, None, 1)
                    .await
                    .map_err(kernel_error)?
                    .activities
                    .is_empty()
                {
                    return Err(conflict(
                        "Bot has a native background activity; finish it before archiving",
                    ));
                }
            }
            if !page.has_more {
                break;
            }
            before = page.next;
        }
        Ok(())
    }

    /// Resolve the conversation a client should open for this Bot.
    ///
    /// The most recently used local root wins; when the Bot has none (first
    /// open, or its conversation was deleted through ordinary session
    /// management) a fresh local root is created. Retired conversations stay
    /// read-only because their rows keep `owner.bot`.
    pub async fn ensure_named_bot_session(&self, id: Uuid) -> Result<BotSession, AppError> {
        let _gate = self.bot_creation_gate().lock_owned().await;
        let bot = self.bot_or_not_found(id).await?;
        if bot.archived {
            return Err(AppError::BotNotFound(
                "Bot is archived; inspect its history or select an active Bot".into(),
            ));
        }
        let session = match self.default_bot_conversation(id).await? {
            Some(session) => session,
            None => {
                self.create_bot_conversation(id, SessionSurface::Local, &bot.name)
                    .await?
            }
        };
        Ok(BotSession::new(session, &bot))
    }

    /// Open the oldest active Bot, creating the default Bot when none exists.
    pub async fn ensure_default_bot_profile(&self) -> Result<BotProfile, AppError> {
        let _gate = self.bot_creation_gate().lock_owned().await;
        match self.default_bot().await? {
            Some(bot) => Ok(bot),
            None => {
                if self.bots().count().await.map_err(repository_error)? > 0 {
                    return Err(AppError::BotNotFound(
                        "No active Bots; create a named Bot to start a conversation".into(),
                    ));
                }
                self.create_default_bot().await
            }
        }
    }

    async fn create_default_bot(&self) -> Result<BotProfile, AppError> {
        let now = chrono::Utc::now().timestamp_millis();
        let name = agena_assistant::AssistantPreferences::default().name;
        let candidate = BotProfile {
            id: Uuid::new_v4(),
            name,
            persona: BotPersona::default(),
            revision: 1,
            archived: false,
            created_at_ms: now,
            updated_at_ms: now,
        };
        // Two clients may race the very first open. The unique active-name
        // index decides the winner; the loser reads back the stored Bot.
        match self.bots().create(&record(&candidate)).await {
            Ok(created) => {
                let created = profile(created);
                self.create_bot_conversation(created.id, SessionSurface::Local, &created.name)
                    .await?;
                Ok(created)
            }
            Err(BotRepositoryError::NameTaken) => self
                .bots()
                .list(false)
                .await
                .map_err(repository_error)?
                .into_iter()
                .next()
                .map(profile)
                .ok_or_else(|| {
                    AppError::BotNotFound(
                        "No active Bots; create a named Bot to start a conversation".into(),
                    )
                }),
            Err(error) => Err(repository_error(error)),
        }
    }

    /// Create a new local conversation for this Bot and return it.
    ///
    /// This is the explicit "new conversation" action: unlike
    /// [`Self::ensure_named_bot_session`] it never reuses an existing one. The
    /// Bot must be active, and the conversation starts empty with a title the
    /// user can rename afterwards.
    pub async fn create_bot_conversation_for(
        &self,
        id: Uuid,
        title: Option<String>,
    ) -> Result<BotSession, AppError> {
        let _gate = self.bot_creation_gate().lock_owned().await;
        let bot = self.bot_or_not_found(id).await?;
        if bot.archived {
            return Err(AppError::BotNotFound(
                "Bot is archived; select an active Bot to start a conversation".into(),
            ));
        }
        let title = match title {
            Some(title) => {
                let title = title.trim();
                if title.is_empty() || title.chars().count() > 512 {
                    return Err(invalid("conversation title must contain 1..512 characters"));
                }
                title.to_owned()
            }
            None => bot.name.clone(),
        };
        let session = self
            .create_bot_conversation(id, SessionSurface::Local, &title)
            .await?;
        Ok(BotSession::new(session, &bot))
    }

    pub async fn ensure_bot_session(&self) -> Result<i64, AppError> {
        let bot = self.ensure_default_bot_profile().await?;
        self.ensure_named_bot_session(bot.id)
            .await
            .map(|session| session.session_id)
    }

    /// The Bot owning a conversation, active or archived.
    pub async fn bot_profile_for_session(
        &self,
        source: i64,
    ) -> Result<Option<BotProfile>, AppError> {
        match self.bot_owner_of(source).await? {
            Some((profile, _)) => Ok(Some(profile)),
            None => Ok(None),
        }
    }

    /// Admission check before any transcript or model mutation.
    ///
    /// Archived Bot history is read-only, and ordinary project traffic never
    /// touches the Bot table.
    pub(in crate::session::manager) async fn require_writable_bot_history(
        &self,
        source: i64,
    ) -> Result<(), AppError> {
        if let Some((profile, _)) = self.bot_owner_of(source).await?
            && profile.archived
        {
            return Err(conflict(
                "Archived Bot history is read-only; select an active Bot or create one",
            ));
        }
        Ok(())
    }

    /// True when this conversation belongs to a Bot, archived or not.
    pub async fn is_reserved_bot_session(&self, source: i64) -> Result<bool, AppError> {
        Ok(self.session_owner(source).await?.is_some())
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
