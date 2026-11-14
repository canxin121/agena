//! Bot identities in their own table, and ownership projected onto sessions.
//!
//! Each Bot has one pinned local Web/TUI conversation and independently scoped
//! remote conversations. Legacy local roots keep their ownership and history.
//! The kernel owns the atomic local mapping; this module shares its lookup and
//! admission rules across HTTP, HostClient and the native clients.
use super::*;
use agena_assistant::bot::*;
use agena_domain::SessionOwner;
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
        local_session_id: None,
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
    /// Serializes Bot identity request receipts. Local conversation uniqueness
    /// is enforced by the kernel transaction and does not depend on this gate.
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

    /// Resolve/create the canonical local root in one kernel transaction.
    async fn ensure_local_bot_conversation(&self, bot: Uuid, title: &str) -> Result<i64, AppError> {
        let current = self
            .kernel()
            .reader
            .local_bot_session(bot)
            .await
            .map_err(kernel_error)?;
        if current.established
            && let Some(session) = current.session
        {
            return Ok(session.get());
        }
        // Retrying an uncertain commit uses this key; a later open gets a
        // fresh key so deleting the canonical root can create its replacement.
        let command = request_key("bot-conversation", &format!("{bot}:{}", Uuid::new_v4()));
        persist(|| {
            self.kernel()
                .sessions
                .ensure_local_bot_session(command, bot, title.to_owned())
        })
        .await
        .map(|receipt| receipt.value.id.get())
        .map_err(kernel_error)
    }

    /// Read the pinned root (or the same legacy candidate first-open adopts).
    pub(super) async fn default_bot_conversation(
        &self,
        bot: Uuid,
    ) -> Result<Option<i64>, AppError> {
        let local = self
            .kernel()
            .reader
            .local_bot_session(bot)
            .await
            .map_err(kernel_error)?;
        Ok(local.session.map(|session| session.get()))
    }

    async fn profile_with_local_session(
        &self,
        mut profile: BotProfile,
    ) -> Result<BotProfile, AppError> {
        profile.local_session_id = self.default_bot_conversation(profile.id).await?;
        Ok(profile)
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
            Some((_, result)) => Ok(Some(
                self.profile_with_local_session(profile(
                    serde_json::from_str(&result).map_err(AppError::from)?,
                ))
                .await?,
            )),
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
        let bot = self
            .bots()
            .list(false)
            .await
            .map_err(repository_error)?
            .into_iter()
            .next()
            .map(profile);
        match bot {
            Some(bot) => self.profile_with_local_session(bot).await.map(Some),
            None => Ok(None),
        }
    }

    async fn bot_or_not_found(&self, id: Uuid) -> Result<BotProfile, AppError> {
        let bot = self
            .bots()
            .get(id)
            .await
            .map(profile)
            .map_err(repository_error)?;
        self.profile_with_local_session(bot).await
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
        let mut items = Vec::with_capacity(records.len());
        for record in records {
            items.push(self.profile_with_local_session(profile(record)).await?);
        }
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
            local_session_id: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let created = self
            .bots()
            .create(&record(&candidate))
            .await
            .map_err(repository_error)?;
        let mut created = profile(created);
        // The Bot's first conversation is an ordinary local root owned by it.
        // Created after the identity row so a crash leaves an identity with no
        // conversation, which `ensure_named_bot_session` repairs on open.
        created.local_session_id = Some(
            self.ensure_local_bot_conversation(created.id, &created.name)
                .await?,
        );
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
        let updated = self.profile_with_local_session(profile(updated)).await?;
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
        let archived = self.profile_with_local_session(profile(archived)).await?;
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
    /// The first legacy root is adopted once. A deleted canonical root is
    /// replaced with a fresh conversation; older local histories stay retired.
    pub async fn ensure_named_bot_session(&self, id: Uuid) -> Result<BotSession, AppError> {
        let _gate = self.bot_creation_gate().lock_owned().await;
        let bot = self.bot_or_not_found(id).await?;
        if bot.archived {
            return Err(AppError::BotNotFound(
                "Bot is archived; inspect its history or select an active Bot".into(),
            ));
        }
        let session = self.ensure_local_bot_conversation(id, &bot.name).await?;
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
            local_session_id: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        // Two clients may race the very first open. The unique active-name
        // index decides the winner; the loser reads back the stored Bot.
        match self.bots().create(&record(&candidate)).await {
            Ok(created) => {
                let mut created = profile(created);
                created.local_session_id = Some(
                    self.ensure_local_bot_conversation(created.id, &created.name)
                        .await?,
                );
                Ok(created)
            }
            Err(BotRepositoryError::NameTaken) => self.default_bot().await?.ok_or_else(|| {
                AppError::BotNotFound(
                    "No active Bots; create a named Bot to start a conversation".into(),
                )
            }),
            Err(error) => Err(repository_error(error)),
        }
    }

    /// Open the one local conversation, creating it when absent. A supplied
    /// title applies only to creation, so retries preserve an existing title.
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
        let session = self.ensure_local_bot_conversation(id, &title).await?;
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
        let Some((profile, owner)) = self.bot_owner_of(source).await? else {
            return Ok(());
        };
        if profile.archived {
            return Err(conflict(
                "Archived Bot history is read-only; select an active Bot or create one",
            ));
        }
        if owner.surface.is_local() {
            let canonical = self
                .ensure_local_bot_conversation(profile.id, &profile.name)
                .await?;
            let mut current = source;
            // Internal workers may run beneath the canonical root. Every
            // intermediate edge must be a Subagent edge: an old user branch
            // must not become writable through one of its retained workers.
            for _ in 0..128 {
                if current == canonical {
                    return Ok(());
                }
                let configuration = self
                    .kernel()
                    .reader
                    .configuration(session_id(current)?)
                    .await
                    .map_err(kernel_error)?;
                if !configuration.relation_kind.is_subagent()
                    || !configuration
                        .owner
                        .as_ref()
                        .is_some_and(|owner| owner.bot == profile.id && owner.surface.is_local())
                {
                    break;
                }
                let Some(parent) = configuration.parent_session_id else {
                    break;
                };
                current = parent.get();
            }
            return Err(AppError::BotLocalHistory {
                canonical_session_id: canonical,
            });
        }
        Ok(())
    }

    /// A local Bot uses one entrance. Rewind-in-place and clear preserve it;
    /// branching would create a second local conversation.
    pub(in crate::session::manager) async fn require_bot_branch_allowed(
        &self,
        source: i64,
    ) -> Result<(), AppError> {
        self.require_writable_bot_history(source).await?;
        if self
            .session_owner(source)
            .await?
            .is_some_and(|owner| owner.surface.is_local())
        {
            return Err(conflict(
                "A Bot has one local conversation; rewind in place or clear its context to continue",
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
