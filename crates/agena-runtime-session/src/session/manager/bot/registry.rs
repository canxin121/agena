//! One atomic JSON registry and durable native creation receipts. The short
//! instance gate is shared by independently constructed manager handles.
use super::*;
use agena_assistant::bot::*;
use caseless::Caseless;
use uuid::Uuid;

const REGISTRY: &str = "bot_registry_v1";
const UPDATE_RECEIPTS: usize = 256;
const RETIRED_SESSIONS: usize = 4096;
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    bots: Vec<BotProfile>,
    retired_sessions: Vec<i64>,
    receipts: HashMap<String, Receipt>,
    pending: Option<Creation>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    payload: Value,
    result: BotProfile,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Creation {
    profile: BotProfile,
    command: RequestId,
    title: String,
    receipt: Option<(String, Value)>,
}
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
fn normalized(name: &str) -> String {
    name.trim().chars().default_case_fold().collect()
}
impl Registry {
    fn default_profile(&self) -> Option<BotProfile> {
        self.bots.iter().find(|p| !p.archived).cloned()
    }
    fn profile(&self, id: Uuid) -> Result<BotProfile, AppError> {
        self.bots
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| {
                AppError::BotNotFound(
                    "Bot was not found; select an existing Bot or create one".into(),
                )
            })
    }
    fn validate_name(
        &self,
        id: Option<Uuid>,
        name: &str,
        persona: &BotPersona,
    ) -> Result<String, AppError> {
        if name.chars().any(char::is_control) {
            return Err(invalid("Bot names cannot contain control characters"));
        }
        let name = name.trim();
        persona
            .validate(name)
            .map_err(|e| invalid(&e.to_string()))?;
        if self
            .bots
            .iter()
            .any(|p| !p.archived && Some(p.id) != id && normalized(&p.name) == normalized(name))
        {
            return Err(conflict("an active Bot already has this name"));
        }
        Ok(name.into())
    }
    fn trim_update_receipts(&mut self, newest: &str) {
        let count = self
            .receipts
            .keys()
            .filter(|key| key.starts_with("update:"))
            .count();
        let mut updates: Vec<_> = self
            .receipts
            .iter()
            .filter(|(key, _)| key.starts_with("update:") && key.as_str() != newest)
            .map(|(key, r)| (r.result.updated_at_ms, key.clone()))
            .collect();
        updates.sort();
        let excess = count.saturating_sub(UPDATE_RECEIPTS);
        for (_, key) in updates.into_iter().take(excess) {
            self.receipts.remove(&key);
        }
    }
    fn replay(&self, key: &str, payload: &Value) -> Result<Option<BotProfile>, AppError> {
        match self.receipts.get(key) {
            Some(receipt) if receipt.payload != *payload => Err(conflict(
                "request_key belongs to a different Bot operation payload",
            )),
            Some(receipt) => Ok(Some(receipt.result.clone())),
            None => Ok(None),
        }
    }
}
impl SessionManager {
    fn bot_registry_gate(&self) -> Arc<tokio::sync::Mutex<()>> {
        card_lock(format!(
            "bot-registry:{}",
            self.kernel_runtime.instance_state.path().display()
        ))
    }
    async fn save_registry(&self, registry: &Registry) -> Result<(), AppError> {
        self.save_bot_state(REGISTRY, registry).await
    }

    async fn finish_bot_creation(&self, registry: &mut Registry) -> Result<(), AppError> {
        let Some(creation) = &registry.pending else {
            return Ok(());
        };
        let session = persist(|| {
            self.kernel().sessions.create_global(
                creation.command,
                creation.title.clone(),
                SessionConfiguration::default(),
            )
        })
        .await
        .map_err(kernel_error)?
        .value
        .id
        .get();
        let creation = registry.pending.take().expect("pending creation");
        let mut profile = creation.profile;
        profile.session_id = session;
        if let Some(existing) = registry.bots.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile.clone();
        } else {
            registry.bots.push(profile.clone());
        }
        if let Some((key, payload)) = creation.receipt {
            registry.receipts.insert(
                key,
                Receipt {
                    payload,
                    result: profile,
                },
            );
        }
        self.save_registry(registry).await
    }
    async fn read_registry(&self) -> Result<Option<Registry>, AppError> {
        let Some(value) = self.bot_state::<Value>(REGISTRY).await? else {
            return Ok(None);
        };
        serde_json::from_value(value).map(Some).map_err(|error| {
            invalid(&format!("Invalid Bot registry ({error}); run the one-time migration before starting the application"))
        })
    }
    async fn existing_registry(&self) -> Result<Option<Registry>, AppError> {
        let Some(mut registry) = self.read_registry().await? else {
            return Ok(None);
        };
        self.finish_bot_creation(&mut registry).await?;
        Ok(Some(registry))
    }
    async fn require_registry(&self) -> Result<Registry, AppError> {
        self.existing_registry().await?.ok_or_else(|| {
            AppError::BotNotFound(
                "Bot was not found; open the Bot entry or create a named Bot".into(),
            )
        })
    }
    async fn load_registry(&self) -> Result<Registry, AppError> {
        let mut registry = self.existing_registry().await?.unwrap_or_default();
        if registry.bots.is_empty() {
            let now = chrono::Utc::now().timestamp_millis();
            let id = Uuid::new_v4();
            let profile = BotProfile {
                id,
                name: agena_assistant::AssistantPreferences::default().name,
                persona: BotPersona::default(),
                session_id: 0,
                revision: 1,
                archived: false,
                created_at_ms: now,
                updated_at_ms: now,
            };
            registry.pending = Some(Creation {
                profile,
                command: request_key("default-bot-create", &id.to_string()),
                title: "Bot".into(),
                receipt: None,
            });
            self.save_registry(&registry).await?;
            self.finish_bot_creation(&mut registry).await?;
        }
        Ok(registry)
    }
    pub async fn list_bot_profiles(&self, include_archived: bool) -> Result<BotList, AppError> {
        let _gate = self.bot_registry_gate().lock_owned().await;
        let registry = self.load_registry().await?;
        Ok(BotList {
            default_bot_id: registry.default_profile().map(|p| p.id),
            items: registry
                .bots
                .into_iter()
                .filter(|p| include_archived || !p.archived)
                .collect(),
        })
    }
    pub async fn get_bot_profile(&self, id: Uuid) -> Result<BotProfile, AppError> {
        let _gate = self.bot_registry_gate().lock_owned().await;
        self.require_registry().await?.profile(id)
    }
    pub async fn bot_profile_for_session(
        &self,
        source: i64,
    ) -> Result<Option<BotProfile>, AppError> {
        let _gate = self.bot_registry_gate().lock_owned().await;
        let registry = match self.existing_registry().await? {
            Some(registry) => registry,
            None => return Ok(None),
        };
        Ok(registry
            .bots
            .into_iter()
            .find(|p| !p.archived && p.session_id == source))
    }
    pub async fn create_bot_profile(&self, create: BotCreate) -> Result<BotProfile, AppError> {
        validate_key(&create.request_key)?;
        Registry::default().validate_name(
            None,
            &create.name,
            &create.persona.clone().unwrap_or_default(),
        )?;
        let _gate = self.bot_registry_gate().lock_owned().await;
        let mut registry = self.load_registry().await?;
        let key = format!("create:{}", create.request_key);
        let payload = serde_json::to_value(&create)?;
        if let Some(profile) = registry.replay(&key, &payload)? {
            return Ok(profile);
        }
        if registry.bots.len() >= 256 {
            return Err(invalid(
                "Bot registry limit is 256; archived histories also count",
            ));
        }
        let persona = create.persona.unwrap_or_default();
        let name = registry.validate_name(None, &create.name, &persona)?;
        let now = chrono::Utc::now().timestamp_millis();
        let profile = BotProfile {
            id: Uuid::new_v4(),
            name,
            persona,
            session_id: 0,
            revision: 1,
            archived: false,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let id = profile.id;
        registry.pending = Some(Creation {
            title: profile.name.clone(),
            profile,
            command: request_key("named-bot-create", &id.to_string()),
            receipt: Some((key, payload)),
        });
        self.save_registry(&registry).await?;
        self.finish_bot_creation(&mut registry).await?;
        registry.profile(id)
    }
    pub async fn update_bot_profile(
        &self,
        id: Uuid,
        update: BotUpdate,
    ) -> Result<BotProfile, AppError> {
        validate_key(&update.request_key)?;
        let _gate = self.bot_registry_gate().lock_owned().await;
        let mut registry = self.require_registry().await?;
        let key = format!("update:{id}:{}", update.request_key);
        let payload = serde_json::to_value(&update)?;
        if let Some(profile) = registry.replay(&key, &payload)? {
            return Ok(profile);
        }
        let mut profile = registry.profile(id)?;
        if profile.archived {
            return Err(conflict("archived Bot history is read-only"));
        }
        if profile.revision != update.expected_revision {
            return Err(conflict(
                "stale Bot revision; reload the profile before editing",
            ));
        }
        profile.name = registry.validate_name(Some(id), &update.name, &update.persona)?;
        profile.persona = update.persona;
        profile.revision += 1;
        profile.updated_at_ms = chrono::Utc::now().timestamp_millis();
        *registry
            .bots
            .iter_mut()
            .find(|p| p.id == id)
            .expect("profile") = profile.clone();
        registry.receipts.insert(
            key.clone(),
            Receipt {
                payload,
                result: profile.clone(),
            },
        );
        registry.trim_update_receipts(&key);
        self.save_registry(&registry).await?;
        Ok(profile)
    }
    pub async fn archive_bot_profile(
        &self,
        id: Uuid,
        archive: BotArchive,
    ) -> Result<BotProfile, AppError> {
        validate_key(&archive.request_key)?;
        let _gate = self.bot_registry_gate().lock_owned().await;
        let mut registry = self.require_registry().await?;
        let key = format!("archive:{id}:{}", archive.request_key);
        let payload = serde_json::to_value(&archive)?;
        if let Some(profile) = registry.replay(&key, &payload)? {
            return Ok(profile);
        }
        let mut profile = registry.profile(id)?;
        if profile.archived || profile.revision != archive.expected_revision {
            return Err(conflict("stale or archived Bot; reload the profile"));
        }
        let dispatch = self.session_dispatch(session_id(profile.session_id)?);
        let _submission = dispatch.submission.try_lock().map_err(|_| {
            conflict(
                "Bot is accepting or changing native work; wait until it is idle before archiving",
            )
        })?;
        let native_live = match self
            .kernel()
            .reader
            .control_snapshot(session_id(profile.session_id)?, 1)
            .await
        {
            Ok(snapshot)
                if !snapshot.active_executions.executions.is_empty()
                    || snapshot.active_execution.is_some()
                    || !snapshot.pending_interactions.interactions.is_empty()
                    || snapshot.summary.queued_executions > 0
                    || snapshot.summary.running_executions > 0 =>
            {
                return Err(conflict(
                    "Bot must be idle with no queued executions or pending native interactions before archiving",
                ));
            }
            Ok(_) => true,
            Err(KernelError::NotFound(_)) => false,
            Err(error) => return Err(kernel_error(error)),
        };
        if native_live {
            self.require_bot_source_idle(session_id(profile.session_id)?)
                .await?;
        }
        profile.archived = true;
        profile.revision += 1;
        profile.updated_at_ms = chrono::Utc::now().timestamp_millis();
        *registry
            .bots
            .iter_mut()
            .find(|p| p.id == id)
            .expect("profile") = profile.clone();
        registry.receipts.insert(
            key.clone(),
            Receipt {
                payload,
                result: profile.clone(),
            },
        );
        registry.trim_update_receipts(&key);
        self.save_registry(&registry).await?;
        Ok(profile)
    }
    async fn require_bot_source_idle(&self, session: SessionId) -> Result<(), AppError> {
        if !self
            .kernel()
            .reader
            .active_activities(session, None, 1)
            .await
            .map_err(kernel_error)?
            .activities
            .is_empty()
        {
            return Err(conflict(
                "Bot has a native background activity; finish it before archiving",
            ));
        }
        let mut before = None;
        loop {
            let page = self
                .kernel()
                .reader
                .query_summaries(SummaryQuery {
                    parent_id: Some(session),
                    include_global: true,
                    agents_only: true,
                    before,
                    limit: 128,
                    ..Default::default()
                })
                .await
                .map_err(kernel_error)?;
            for child in &page.sessions {
                if child
                    .agent_status
                    .is_some_and(|status| !status.is_terminal())
                    || child.queued_executions > 0
                    || child.running_executions > 0
                {
                    return Err(conflict(
                        "Bot has an active native Subagent; finish it before archiving",
                    ));
                }
                let control = self
                    .kernel()
                    .reader
                    .control_snapshot(child.id, 1)
                    .await
                    .map_err(kernel_error)?;
                if !control.pending_interactions.interactions.is_empty()
                    || !self
                        .kernel()
                        .reader
                        .active_activities(child.id, None, 1)
                        .await
                        .map_err(kernel_error)?
                        .activities
                        .is_empty()
                {
                    return Err(conflict(
                        "Bot Subagent has pending interaction or native background activity; finish it before archiving",
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
    async fn resolve_bot_profile(
        &self,
        registry: &mut Registry,
        id: Uuid,
    ) -> Result<BotProfile, AppError> {
        let mut profile = registry.profile(id)?;
        if profile.archived {
            return Err(AppError::BotNotFound(
                "Bot is archived; inspect its history or select an active Bot".into(),
            ));
        }
        match self
            .kernel()
            .reader
            .metadata(session_id(profile.session_id)?)
            .await
        {
            Ok(_) => {
                self.require_bot_root(profile.session_id).await?;
                return Ok(profile);
            }
            Err(KernelError::NotFound(_)) => {}
            Err(error) => return Err(kernel_error(error)),
        }
        let previous = profile.session_id;
        if registry.retired_sessions.len() >= RETIRED_SESSIONS {
            return Err(conflict(
                "Bot recovery history limit reached; archive this Bot and create a named Bot",
            ));
        }
        registry.retired_sessions.push(previous);
        profile.revision += 1;
        profile.updated_at_ms = chrono::Utc::now().timestamp_millis();
        registry.pending = Some(Creation {
            title: profile.name.clone(),
            profile,
            command: request_key("named-bot-recovery", &format!("{id}:{previous}")),
            receipt: None,
        });
        self.save_registry(registry).await?;
        self.finish_bot_creation(registry).await?;
        registry.profile(id)
    }
    pub async fn ensure_default_bot_profile(&self) -> Result<BotProfile, AppError> {
        let _gate = self.bot_registry_gate().lock_owned().await;
        let mut registry = self.load_registry().await?;
        let id = registry
            .default_profile()
            .ok_or_else(|| {
                AppError::BotNotFound(
                    "No active Bots; create a named Bot to start a conversation".into(),
                )
            })?
            .id;
        self.resolve_bot_profile(&mut registry, id).await
    }
    pub async fn ensure_named_bot_session(&self, id: Uuid) -> Result<BotSession, AppError> {
        let _gate = self.bot_registry_gate().lock_owned().await;
        let mut registry = self.require_registry().await?;
        Ok(self.resolve_bot_profile(&mut registry, id).await?.into())
    }
    pub async fn ensure_bot_session(&self) -> Result<i64, AppError> {
        Ok(self.ensure_default_bot_profile().await?.session_id)
    }
    /// Called under native admission before any transcript/model mutation.
    /// Ordinary project traffic must not initialize a registry as a side effect.
    pub(in crate::session::manager) async fn require_writable_bot_history(
        &self,
        source: i64,
    ) -> Result<(), AppError> {
        let _gate = self.bot_registry_gate().lock_owned().await;
        if let Some(registry) = self.read_registry().await?
            && (registry
                .bots
                .iter()
                .any(|p| p.session_id == source && p.archived)
                || registry.retired_sessions.contains(&source))
        {
            return Err(conflict(
                "Archived or replaced Bot history is read-only; select an active Bot or create one",
            ));
        }
        Ok(())
    }
    pub async fn is_reserved_bot_session(&self, source: i64) -> Result<bool, AppError> {
        Ok(self.bot_session_ids().await?.contains(&source))
    }
    /// Shared routing/discovery exclusion, including archived and replaced roots.
    pub(super) async fn bot_session_ids(&self) -> Result<std::collections::HashSet<i64>, AppError> {
        let _gate = self.bot_registry_gate().lock_owned().await;
        let Some(registry) = self.existing_registry().await? else {
            return Ok(Default::default());
        };
        Ok(registry
            .bots
            .iter()
            .map(|p| p.session_id)
            .chain(registry.retired_sessions)
            .collect())
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
