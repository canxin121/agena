//! Local AI Bot ownership, chat admission and background binding reconciliation.
use super::*;
use agena_assistant::{
    BindingState, ChannelAccessPolicy, ChannelGrant, ChannelMember, ChannelRole,
    ChannelSessionMode, ChannelThreadMode, ChannelTrigger, CreateIntegration, IntegrationBinding,
    IntegrationControl, IntegrationControlAction, IntegrationMode, IntegrationSpec,
    IntegrationsSnapshot,
    bot::{BotList, BotProfile, BotSession},
};
use agena_plugin_sdk::HostClientExt;
use agena_plugin_sdk::operations::business::{BotListQuery, BusinessCommand, BusinessQuery};
use agena_plugin_sdk::operations::queries::{GetSessionParams, Query};
use futures_util::StreamExt;
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub(in super::super) enum ChatAccessMode {
    #[default]
    Whitelist,
    Blacklist,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(in super::super) struct ChatAccess {
    pub mode: ChatAccessMode,
    pub private_enabled: bool,
    pub group_enabled: bool,
    pub channel_enabled: bool,
    pub chat_ids: BTreeSet<String>,
}

impl ChatAccess {
    pub(super) fn normalize(&mut self) -> Result<()> {
        if self.chat_ids.len() > MAX_ROOMS {
            return Err(invalid("Select at most 1000 Telegram chat IDs"));
        }
        self.chat_ids = self
            .chat_ids
            .iter()
            .map(|id| numeric_chat_id(id))
            .collect::<Result<_>>()?;
        Ok(())
    }

    pub(in super::super) fn allows(&self, chat_id: &str, kind: ConversationKind) -> bool {
        let enabled = match kind {
            ConversationKind::Direct => self.private_enabled,
            ConversationKind::Group => self.group_enabled,
            ConversationKind::Broadcast => self.channel_enabled,
        };
        enabled
            && match self.mode {
                ChatAccessMode::Whitelist => self.chat_ids.contains(chat_id),
                ChatAccessMode::Blacklist => !self.chat_ids.contains(chat_id),
            }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in super::super) struct LocalAiBot {
    pub id: Uuid,
    pub name: String,
    pub archived: bool,
}
impl From<BotProfile> for LocalAiBot {
    fn from(bot: BotProfile) -> Self {
        Self {
            id: bot.id,
            name: bot.name,
            archived: bot.archived,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RoutingState {
    #[serde(default)]
    source_session_id: Option<i64>,
    #[serde(default)]
    paused_by_policy: BTreeMap<String, i64>,
    #[serde(default)]
    last_error: Option<String>,
    #[serde(default)]
    last_reconciled_ms: i64,
}

pub(super) fn admission_clock(
    old: Option<&StoredBot>,
    next: &BotConfiguration,
    rooms: &BTreeMap<String, Room>,
    now: i64,
) -> (i64, BTreeMap<String, i64>) {
    let Some(old) = old else {
        return (now, BTreeMap::new());
    };
    if old.config.ai_bot_id == next.ai_bot_id
        && old.config.chat_access == next.chat_access
        && old.config.enabled == next.enabled
    {
        return (old.access_since_ms, old.chat_since_ms.clone());
    }
    let same_owner = old.config.ai_bot_id == next.ai_bot_id;
    let mut cutoffs = BTreeMap::new();
    // A policy edit only advances newly admitted rooms. Existing allowed
    // conversations retain their unread authorized backlog.
    if same_owner && old.config.enabled && next.enabled {
        for room in rooms.values() {
            if let Some(kind) = room_kind(room)
                && old.config.chat_access.allows(&room.chat_id, kind)
                && next.chat_access.allows(&room.chat_id, kind)
            {
                cutoffs.insert(
                    room.chat_id.clone(),
                    old.chat_since_ms
                        .get(&room.chat_id)
                        .copied()
                        .unwrap_or(old.access_since_ms),
                );
            }
        }
    }
    (now, cutoffs)
}

fn managed_request_key(bot: &ResolvedBot, chat_id: &str) -> Result<String> {
    let ai_bot_id = bot
        .ai_bot_id
        .ok_or_else(|| invalid("Select a local AI Bot"))?;
    Ok(format!(
        "telegram-{}",
        hex::encode(Sha256::digest(
            format!("{}|{}|{}", bot.id, ai_bot_id, chat_id).as_bytes()
        ))
    ))
}

fn routing_key(id: &str) -> String {
    format!("routing:{id}")
}

fn numeric_chat_id(value: &str) -> Result<String> {
    value
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|id| *id != 0)
        .map(|id| id.to_string())
        .ok_or_else(|| invalid("Enter the numeric Telegram chat ID"))
}

fn room_kind(room: &Room) -> Option<ConversationKind> {
    match room.kind.as_str() {
        "direct" => Some(ConversationKind::Direct),
        "group" => Some(ConversationKind::Group),
        "broadcast" => Some(ConversationKind::Broadcast),
        _ => None,
    }
}

impl TelegramPlugin {
    pub(in super::super) async fn local_ai_bots(&self) -> Result<CommandResult> {
        let catalog: BotList = self
            .host()?
            .query_as(BusinessQuery::ListBots(BotListQuery {
                include_archived: false,
            }))
            .await?;
        let bots = catalog
            .items
            .into_iter()
            .filter(|bot| !bot.archived)
            .map(LocalAiBot::from)
            .collect::<Vec<_>>();
        Ok(result("Local AI Bots loaded", json!({ "bots": bots })))
    }

    pub(super) async fn resolve_local_ai_bot(&self, id: Uuid) -> Result<(LocalAiBot, i64)> {
        let profile: BotProfile = self
            .host()?
            .query_as(BusinessQuery::GetBot { bot_id: id })
            .await?;
        if profile.archived {
            return Err(invalid(
                "This local AI Bot is archived; select an active Bot",
            ));
        }
        let session: BotSession = self
            .host()?
            .command_as(BusinessCommand::EnsureBotSession { bot_id: id })
            .await?;
        if session.bot_id != id || session.session_id <= 0 {
            return Err(PluginError::internal(
                "The host returned an invalid AI Bot session",
            ));
        }
        Ok((profile.into(), session.session_id))
    }

    pub(super) async fn routing_error(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .read_record::<RoutingState>(&routing_key(id))
            .await?
            .last_error)
    }

    /// The host checks this binding in the selected AI Bot's scope. Adapter
    /// requests deliberately contain no caller-supplied source-session authority.
    pub(in super::super) async fn require_binding_owner(
        &self,
        bot: &ResolvedBot,
        binding_id: &str,
    ) -> Result<()> {
        // Unit transport fixtures can run without an Application.
        if self.host.get().is_none() {
            return Ok(());
        }
        let ai_bot_id = bot
            .ai_bot_id
            .ok_or_else(|| invalid("Select a local AI Bot for this Telegram bot"))?;
        let source = bot
            .source_session_id
            .ok_or_else(|| invalid("Save this Telegram bot to connect its local AI Bot session"))?;
        let profile: BotProfile = self
            .host()?
            .query_as(BusinessQuery::GetBot { bot_id: ai_bot_id })
            .await?;
        if profile.archived {
            return Err(invalid("The local AI Bot is archived"));
        }
        let binding_id = Uuid::parse_str(binding_id)
            .map_err(|_| invalid("Invalid Telegram binding identity"))?;
        let detail: agena_assistant::IntegrationDetail = self
            .host()?
            .query_as(BusinessQuery::GetIntegration {
                session_id: source,
                binding_id,
            })
            .await?;
        if detail.binding.owner_bot_id != Some(ai_bot_id)
            || detail.binding.source_session_id != source
            || detail.binding.spec.provider != TELEGRAM_PLUGIN_ID
            || !binding_selects_bot(&detail.binding, bot)
        {
            return Err(invalid(
                "This Telegram binding belongs to a different local AI Bot",
            ));
        }
        Ok(())
    }

    pub(in super::super) async fn add_manual_chat(
        &self,
        bot_id: String,
        chat_id: String,
        kind: ConversationKind,
        title: Option<String>,
    ) -> Result<CommandResult> {
        let chat_id = numeric_chat_id(&chat_id)?;
        if (kind == ConversationKind::Direct) != !chat_id.starts_with('-') {
            return Err(invalid(
                "Private chat IDs must be positive; group and channel IDs must be negative",
            ));
        }
        let title = title.unwrap_or_default().trim().to_owned();
        if title.chars().count() > 120 || title.chars().any(char::is_control) {
            return Err(invalid(
                "Chat title must contain at most 120 characters without control characters",
            ));
        }
        let selection = BotSelection {
            bot_id: Some(bot_id),
        };
        let resolved = self.resolve_bot(&selection, None).await?;
        let gate = self.account_lock(&resolved.id);
        let _account = gate.lock().await;
        let resolved = self.resolve_bot(&selection, None).await?;
        let key = stream_key(&resolved);
        let mut stream: Stream = self.read_record(&key).await?;
        if !stream.rooms.contains_key(&chat_id) && stream.rooms.len() >= MAX_ROOMS {
            return Err(invalid("Telegram discovered-chat limit reached"));
        }
        let room = stream.rooms.entry(chat_id.clone()).or_insert_with(|| Room {
            chat_id: chat_id.clone(),
            first_seen_ms: chrono::Utc::now().timestamp_millis(),
            ..Default::default()
        });
        room.kind = kind.as_str().to_owned();
        if !title.is_empty() || room.title.is_empty() {
            room.title = if title.is_empty() { chat_id } else { title };
        }
        let room = room.clone();
        self.write_record(&key, &stream).await?;
        Ok(result(
            "Telegram chat added",
            json!({ "bot_id": resolved.id, "room": room }),
        ))
    }

    pub(in super::super) fn start_background(&self) -> Result<()> {
        let mut task = self.background.lock().unwrap_or_else(|e| e.into_inner());
        if task.as_ref().is_some_and(|task| !task.is_finished()) {
            return Ok(());
        }
        let worker = self.background_worker()?;
        *task = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let registry: Registry = match worker.read_record("bots").await {
                    Ok(registry) => registry,
                    Err(error) => {
                        if let Ok(host) = worker.host() {
                            host.log(
                                agena_plugin_sdk::host_api::LogLevel::Warn,
                                "Telegram discovery could not read its bot registry".into(),
                                json!({ "error": error.to_string() }),
                            )
                            .await;
                        }
                        continue;
                    }
                };
                futures_util::stream::iter(registry.bots.into_keys())
                    .for_each_concurrent(4, |id| {
                        let worker = &worker;
                        async move {
                            let _ = worker.synchronize_managed_bot(&id, true).await;
                        }
                    })
                    .await;
            }
        }));
        Ok(())
    }

    fn background_worker(&self) -> Result<Self> {
        let worker = Self {
            client: OnceLock::new(),
            host: OnceLock::new(),
            registry_lock: self.registry_lock.clone(),
            account_locks: self.account_locks.clone(),
            settings: PluginSettings::new(),
            identities: self.identities.clone(),
            background: Mutex::new(None),
        };
        worker
            .client
            .set(self.client().clone())
            .map_err(|_| PluginError::internal("Telegram client already initialized"))?;
        worker
            .host
            .set(self.host()?.clone())
            .map_err(|_| PluginError::internal("Telegram host already initialized"))?;
        worker.settings.set(
            self.settings
                .get()
                .cloned()
                .ok_or_else(|| PluginError::internal("Telegram plugin was not initialized"))?,
            "Telegram settings already initialized",
        )?;
        Ok(worker)
    }

    pub(in super::super) fn stop_background(&self) {
        if let Some(task) = self
            .background
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
    }

    /// Reuses the durable account collector; no independent getUpdates consumer
    /// or offset exists for discovery. The routing lock serializes reconciliation
    /// without holding the account lock while invoking host integration services.
    pub(super) async fn synchronize_managed_bot(&self, id: &str, fetch: bool) -> Result<()> {
        let gate = self.account_lock(&format!("routing:{id}"));
        let _routing = gate.lock().await;
        let mut state: RoutingState = self.read_record(&routing_key(id)).await?;
        let outcome = self.reconcile_bot(id, fetch, &mut state).await;
        state.last_reconciled_ms = chrono::Utc::now().timestamp_millis();
        state.last_error = outcome.as_ref().err().map(ToString::to_string);
        self.write_record(&routing_key(id), &state).await?;
        outcome
    }

    async fn reconcile_bot(&self, id: &str, fetch: bool, state: &mut RoutingState) -> Result<()> {
        let selection = BotSelection {
            bot_id: Some(id.to_owned()),
        };
        let mut resolved = self.resolve_bot(&selection, None).await?;
        let ai_bot_id = resolved
            .ai_bot_id
            .ok_or_else(|| invalid("Select a local AI Bot"))?;
        let profile: BotProfile = self
            .host()?
            .query_as(BusinessQuery::GetBot { bot_id: ai_bot_id })
            .await?;
        if profile.archived {
            return Err(invalid("The selected local AI Bot is archived"));
        }
        if profile.local_session_id.is_none()
            || profile.local_session_id != resolved.source_session_id
        {
            let (ai_bot, source) = self.resolve_local_ai_bot(ai_bot_id).await?;
            let gate = self.account_lock(id);
            let _account = gate.lock().await;
            let _registry = self.registry_lock.lock().await;
            let mut registry: Registry = self.read_record("bots").await?;
            let stored = registry
                .bots
                .get_mut(id)
                .ok_or_else(|| invalid("Telegram bot was removed"))?;
            if stored.config.ai_bot_id != Some(ai_bot_id) {
                return Err(invalid(
                    "Telegram AI Bot selection changed; retry synchronization",
                ));
            }
            if stored.source_session_id != Some(source) {
                stored.source_session_id = Some(source);
                stored.ai_bot = Some(ai_bot);
                // A new canonical session starts a new route. Retained messages
                // from the retired route must not replay into the new one.
                stored.access_since_ms = chrono::Utc::now().timestamp_millis();
                stored.chat_since_ms.clear();
            }
            resolved = stored.resolved();
            self.write_record("bots", &registry).await?;
        }
        let rooms = {
            let gate = self.account_lock(id);
            let _account = gate.lock().await;
            let current = self.resolve_bot(&selection, None).await?;
            let mut stream: Stream = self.read_record(&stream_key(&current)).await?;
            if fetch && current.enabled {
                self.bot_identity(&current.connection, false).await?;
                self.collect_updates(&current, &mut stream, false).await?;
            }
            stream.rooms
        };
        let source = resolved.source_session_id.ok_or_else(|| {
            invalid("Select and save a local AI Bot before enabling Telegram chats")
        })?;
        if let Some(previous_source) = state
            .source_session_id
            .filter(|previous| *previous != source)
            && self.source_session_exists(previous_source).await?
        {
            let previous: IntegrationsSnapshot = self
                .host()?
                .query_as(BusinessQuery::ListIntegrations {
                    session_id: previous_source,
                })
                .await?;
            for binding in previous
                .bindings
                .iter()
                .filter(|binding| binding_selects_bot(binding, &resolved))
            {
                if binding.state == BindingState::Enabled {
                    self.pause_by_policy(id, previous_source, binding, state)
                        .await?;
                }
            }
        }
        state.source_session_id = Some(source);
        let snapshot: IntegrationsSnapshot = self
            .host()?
            .query_as(BusinessQuery::ListIntegrations { session_id: source })
            .await?;
        let mut bindings = snapshot
            .bindings
            .into_iter()
            .filter(|binding| binding_selects_bot(binding, &resolved))
            .collect::<Vec<_>>();

        for binding in &mut bindings {
            let Some(chat_id) = binding.spec.resource.single() else {
                continue;
            };
            let kind = rooms.get(chat_id).and_then(room_kind).or_else(|| {
                binding.spec.channel_policy.as_ref().and_then(|policy| {
                    (policy.kinds.len() == 1)
                        .then(|| policy.kinds.iter().next().copied())
                        .flatten()
                })
            });
            let allowed = resolved.enabled
                && binding.source_session_id == source
                && kind.is_some_and(|kind| resolved.chat_access.allows(chat_id, kind));
            let key = binding.id.to_string();
            if !allowed && binding.state == BindingState::Enabled {
                *binding = self.pause_by_policy(id, source, binding, state).await?;
            } else if allowed
                && binding.state == BindingState::Paused
                && state.paused_by_policy.get(&key) == Some(&binding.revision)
            {
                *binding = self
                    .control_managed_binding(
                        source,
                        binding,
                        IntegrationControlAction::Enable,
                        Some(resolved.admission_since_ms(chat_id).div_euclid(1000) * 1000),
                    )
                    .await?;
                state.paused_by_policy.remove(&key);
            } else if allowed
                && binding.state == BindingState::Disabled
                && binding.request_key == managed_request_key(&resolved, chat_id)?
            {
                // Complete an interrupted create/enable pair after restart.
                *binding = self
                    .control_managed_binding(
                        source,
                        binding,
                        IntegrationControlAction::Enable,
                        Some(resolved.admission_since_ms(chat_id).div_euclid(1000) * 1000),
                    )
                    .await?;
            } else if state
                .paused_by_policy
                .get(&key)
                .is_some_and(|revision| *revision != binding.revision)
            {
                // An operator changed this binding after the policy pause.
                state.paused_by_policy.remove(&key);
            }
        }
        if !resolved.enabled {
            return Ok(());
        }

        let mut created = 0;
        for room in rooms.values() {
            let Some(kind) = room_kind(room) else {
                continue;
            };
            if !resolved.chat_access.allows(&room.chat_id, kind)
                || bindings.iter().any(|binding| {
                    binding.source_session_id == source
                        && binding.spec.resource.matches(&room.chat_id)
                        && binding
                            .spec
                            .channel_policy
                            .as_ref()
                            .is_some_and(|policy| policy.serves(kind))
                })
            {
                continue;
            }
            // Bound each pass; remaining discovered chats are handled next tick.
            if created >= 32 {
                break;
            }
            let request =
                default_binding(&resolved, room, kind, chrono::Utc::now().timestamp_millis())?;
            let binding: IntegrationBinding = self
                .host()?
                .command_as(BusinessCommand::CreateIntegration {
                    session_id: source,
                    request,
                })
                .await?;
            let binding = if binding.state == BindingState::Disabled {
                self.control_managed_binding(
                    source,
                    &binding,
                    IntegrationControlAction::Enable,
                    Some(resolved.admission_since_ms(&room.chat_id).div_euclid(1000) * 1000),
                )
                .await?
            } else {
                binding
            };
            bindings.push(binding);
            created += 1;
        }
        Ok(())
    }

    async fn source_session_exists(&self, source: i64) -> Result<bool> {
        match self
            .host()?
            .query_as::<Value>(Query::GetSession(GetSessionParams { session_id: source }))
            .await
        {
            Ok(_) => Ok(true),
            Err(error) if error.failure.category == agena_failure::FailureCategory::NotFound => {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    async fn pause_by_policy(
        &self,
        bot_id: &str,
        source: i64,
        binding: &IntegrationBinding,
        state: &mut RoutingState,
    ) -> Result<IntegrationBinding> {
        // Keep the journal across AI Bot changes so switching back can resume
        // only routes paused by this policy, while operator pauses stay paused.
        let key = binding.id.to_string();
        state
            .paused_by_policy
            .insert(key.clone(), binding.revision.saturating_add(1));
        self.write_record(&routing_key(bot_id), state).await?;
        let paused = match self
            .control_managed_binding(source, binding, IntegrationControlAction::Pause, None)
            .await
        {
            Ok(paused) => paused,
            Err(error) => {
                state.paused_by_policy.remove(&key);
                return Err(error);
            }
        };
        state.paused_by_policy.insert(key, paused.revision);
        Ok(paused)
    }

    async fn control_managed_binding(
        &self,
        source: i64,
        binding: &IntegrationBinding,
        action: IntegrationControlAction,
        not_before_ms: Option<i64>,
    ) -> Result<IntegrationBinding> {
        self.host()?
            .command_as(BusinessCommand::ControlIntegration {
                session_id: source,
                binding_id: binding.id,
                request: IntegrationControl {
                    expected_revision: binding.revision,
                    action,
                    not_before_ms,
                },
            })
            .await
    }
}

fn binding_selects_bot(binding: &IntegrationBinding, bot: &ResolvedBot) -> bool {
    binding.spec.provider == TELEGRAM_PLUGIN_ID
        && (binding.spec.config.get("bot_id").and_then(Value::as_str) == Some(bot.id.as_str())
            || (bot.id == "default"
                && binding.spec.config.get("bot_id").is_none()
                && binding.spec.channel_policy.as_ref().is_some_and(|policy| {
                    policy.identity.tenant == TENANT
                        && policy
                            .identity
                            .account
                            .eq_ignore_ascii_case(&bot.connection.account)
                })))
}

pub(super) fn default_binding(
    bot: &ResolvedBot,
    room: &Room,
    kind: ConversationKind,
    _now: i64,
) -> Result<CreateIntegration> {
    let ai_bot_id = bot
        .ai_bot_id
        .ok_or_else(|| invalid("Select a local AI Bot"))?;
    let local_owner = format!("agena:bot:{ai_bot_id}");
    let mut members = vec![ChannelMember {
        actor: local_owner,
        role: ChannelRole::Owner,
        expires_at_ms: None,
    }];
    // Telegram private-chat actors equal the positive chat ID. Channel posts
    // use the adapter's authenticated channel:<id> actor. Group users remain
    // ordinary Members; no remote sender is promoted to local owner.
    if kind != ConversationKind::Group {
        members.push(ChannelMember {
            actor: if kind == ConversationKind::Direct {
                room.chat_id.clone()
            } else {
                format!("channel:{}", room.chat_id)
            },
            role: ChannelRole::Member,
            expires_at_ms: None,
        });
    }
    let request_key = managed_request_key(bot, &room.chat_id)?;
    Ok(CreateIntegration {
        request_key,
        spec: IntegrationSpec {
            title: if room.title.trim().is_empty() {
                format!("Telegram {}", room.chat_id)
            } else {
                room.title.chars().take(120).collect()
            },
            provider: TELEGRAM_PLUGIN_ID.into(),
            mode: IntegrationMode::Channel,
            config: json!({ "bot_id": bot.id }),
            resource: room.chat_id.clone().into(),
            thread: None,
            event_types: vec!["message".into()],
            actors: vec![],
            instruction: String::new(),
            poll_seconds: 2,
            expires_at_ms: 0,
            daily_limit: 0,
            total_limit: 0,
            max_execution_seconds: 300,
            use_source_permissions: true,
            channel_policy: Some(ChannelAccessPolicy {
                identity: ChannelIdentity {
                    tenant: TENANT.into(),
                    account: bot.connection.account.clone(),
                },
                kinds: BTreeSet::from([kind]),
                session_mode: ChannelSessionMode::PerConversation,
                thread_mode: if kind == ConversationKind::Group {
                    ChannelThreadMode::PerThread
                } else {
                    ChannelThreadMode::Exact
                },
                trigger: if kind == ConversationKind::Group {
                    ChannelTrigger::DirectOrMention
                } else {
                    ChannelTrigger::EveryMessage
                },
                members,
                allow_unlisted_chat: kind == ConversationKind::Group,
                grant: ChannelGrant::default(),
                per_user_daily_limit: 0,
                allow_automated_reports: false,
                service_grant: ChannelGrant::default(),
            }),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_rules_apply_to_new_chats_without_a_discovery_snapshot() {
        let mut access = ChatAccess {
            mode: ChatAccessMode::Blacklist,
            private_enabled: true,
            group_enabled: true,
            channel_enabled: false,
            chat_ids: BTreeSet::from(["-8".into(), "12".into()]),
        };
        assert!(access.allows("-99", ConversationKind::Group));
        assert!(access.allows("77", ConversationKind::Direct));
        assert!(!access.allows("-8", ConversationKind::Group));
        assert!(!access.allows("12", ConversationKind::Direct));
        assert!(!access.allows("-99", ConversationKind::Broadcast));
        access.mode = ChatAccessMode::Whitelist;
        assert!(!access.allows("-99", ConversationKind::Group));
        assert!(access.allows("-8", ConversationKind::Group));
        access.group_enabled = false;
        assert!(!access.allows("-8", ConversationKind::Group));
    }

    #[test]
    fn chat_ids_remain_exact_signed_integer_strings() {
        let mut access = ChatAccess {
            chat_ids: BTreeSet::from([" -1000000000001 ".into(), "0012".into()]),
            ..Default::default()
        };
        access.normalize().unwrap();
        assert_eq!(
            access.chat_ids,
            BTreeSet::from(["-1000000000001".into(), "12".into()])
        );
        for bad in ["0", "@name", "12.5", "9223372036854775808"] {
            assert!(numeric_chat_id(bad).is_err());
        }
    }
}
