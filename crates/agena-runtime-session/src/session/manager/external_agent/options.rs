use std::collections::BTreeSet;

use agena_domain::{
    ExternalAgentOption, ExternalAgentOptionChange, ExternalAgentOptionKind as Kind,
    ExternalAgentOptionType, ExternalAgentOptionValue, ExternalAgentOptions,
};
use agena_storage::kernel::ExternalAgentSelectionChange;

use super::*;

impl SessionManager {
    pub(in crate::session::manager) async fn inspect_external_agent_options(
        &self,
        id: i64,
        change: Option<ExternalAgentOptionChange>,
        preview: bool,
    ) -> Result<ExternalAgentOptions, AppError> {
        if preview && change.is_some() {
            return Err(AppError::Config(
                "An ACP startup preview cannot change session options.".into(),
            ));
        }
        let session = session_id(id)?;
        self.require_writable_bot_history(id).await?;
        let configuration = read_with_busy_retry(session, "external_agent_options", || {
            self.kernel().reader.configuration(session)
        })
        .await?;
        let agent = configuration
            .external_agent
            .clone()
            .ok_or_else(|| AppError::Config("This session has no external agent.".into()))?;
        let spec = self.resolve_external_agent(&agent)?;
        let expected = &configuration.external_agent_selection;
        let link = configuration.native_session_link();
        let native_linked = link.is_some();
        if change.is_none()
            && ((!preview && !configuration.external_agent_state.options.is_empty())
                || (preview && native_linked))
        {
            return Ok(ExternalAgentOptions {
                selection: expected.clone(),
                options: configuration.external_agent_state.options.clone(),
                preview_commands: Vec::new(),
            });
        }
        let _permit = self.reserve_execution()?;
        let state = self.state_for_session(session).await?;
        let context = self
            .execution_context_for_execution(session, &state, None)
            .await?;
        let executor = state.executor.for_session_context_async(&context).await;
        let cwd = super::lifecycle::canonical_directory(executor.workspace_root()).await?;
        let additional_directories = super::lifecycle::directories_for_agent(
            &executor,
            &cwd,
            &configuration.external_agent_directories,
        )
        .await?;
        let previous = link
            .as_ref()
            .filter(|prior| prior.cwd == cwd)
            .map(|prior| prior.native_id.as_str());

        let mut selection = expected.clone();
        if let Some(change) = &change {
            match &change.value {
                Some(value) => {
                    selection.insert(change.kind.clone(), value.clone());
                }
                None => {
                    selection.remove(&change.kind);
                }
            }
        }
        let model_changed = expected.get(&Kind::Model) != selection.get(&Kind::Model);
        if model_changed && !selection.contains_key(&Kind::Model) {
            selection.remove(&Kind::ThoughtLevel);
        }
        if !preview && previous.is_some() {
            self.close_idle_external_agent(session).await;
        }
        let cancel = self.shutdown.child_token();
        let mut client = self.spawn_external_agent_with_directories(
            spec.clone(),
            &cwd,
            &additional_directories,
        )?;
        let outcome = async {
            client.prepare_provider_routes(&cancel).await?;
            if preview || previous.is_none() || (change.is_none() && selection.is_empty()) {
                match client
                    .session_options(&cwd, previous, &additional_directories, &cancel)
                    .await
                {
                    Ok(Some(response)) => {
                        let mut advertised = config_options(&response);
                        if preview {
                            let commands = if !native_linked && client.uses_v2() {
                                super::state::available_commands(&response["availableCommands"])
                                    .unwrap_or_default()
                            } else {
                                Vec::new()
                            };
                            return Ok((advertised, commands, None));
                        }
                        if change.is_none() && selection.is_empty() {
                            return Ok((advertised, Vec::new(), None));
                        }
                        if previous.is_none() {
                            if model_changed
                                && let Some(thought) = selection.get(&Kind::ThoughtLevel)
                                && !option_offers(&advertised, &Kind::ThoughtLevel, thought)
                                && selection.get(&Kind::Model).is_none_or(|model| {
                                    option_offers(&advertised, &Kind::Model, model)
                                })
                            {
                                selection.remove(&Kind::ThoughtLevel);
                            }
                            let missing = selection
                                .iter()
                                .find(|(kind, value)| !option_offers(&advertised, kind, value));
                            if let Some((missing_kind, _)) = missing {
                                let parent_changed = selection.iter().any(|(kind, value)| {
                                    kind != missing_kind
                                        && advertised.iter().any(|option| {
                                            &option.kind == kind && option.current_value != *value
                                        })
                                });
                                if !parent_changed {
                                    return Err(AppError::Config(format!(
                                        "{} no longer offers the selected {}. Open its options and choose an available value.",
                                        agent.label(),
                                        missing_kind.label(),
                                    )));
                                }
                            } else {
                                for (kind, value) in &selection {
                                    if let Some(option) =
                                        advertised.iter_mut().find(|option| &option.kind == kind)
                                    {
                                        option.current_value = value.clone();
                                    }
                                }
                                return Ok((advertised, Vec::new(), None));
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                    Err(error) if preview => return Err(error),
                    Err(error) => tracing::debug!(
                        %error,
                        "could not inspect ACP options without opening a session"
                    ),
                }
            }
            if preview {
                return Ok((Vec::new(), Vec::new(), None));
            }
            let session_context = super::lifecycle::session_context(
                agent.clone(),
                session,
                &executor,
                &state,
                &context,
                additional_directories,
                None,
            )
            .await;
            let mut opened = client
                .start_with_context(
                    &cwd,
                    None,
                    previous.map(|id| (id, &configuration.external_agent_state)),
                    false,
                    acp::OpenPurpose::Prompt,
                    &session_context,
                    &cancel,
                )
                .await?;
            let result = async {
                for (kind, value) in selection.clone() {
                    if !opened.offers(&kind, &value) {
                        if change.is_none() {
                            continue;
                        }
                        if kind == Kind::ThoughtLevel && model_changed {
                            selection.remove(&kind);
                            continue;
                        }
                    }
                    client
                        .set_option(&mut opened, &kind, &value, &cancel)
                        .await?;
                }
                Ok::<_, AppError>(())
            }
            .await;
            let mut new_link = None;
            if opened.resumed {
                client.close_native_session_if_supported(&opened.id).await;
            } else if result.is_ok() && client.can_reopen_session() && previous.is_none() {
                client.close_native_session_if_supported(&opened.id).await;
                new_link = Some(ExternalAgentLink {
                    native_id: opened.id.clone(),
                    cwd: cwd.clone(),
                    incarnation: None,
                    resume_at: Some(agena_storage::kernel::ExternalAgentHistoryAnchor {
                        tail: None,
                    }),
                });
            } else {
                client.close_ephemeral(&opened, &cancel).await;
            }
            result?;
            Ok::<_, AppError>((opened.options, Vec::new(), new_link))
        }
        .await;
        self.record_external_agent_memo(&spec, &client);
        self.retire_external_agent_client(client).await;
        let (options, preview_commands, new_link) = outcome?;
        if new_link.is_some() {
            self.invalidate_external_agent_control_plane(&agent, true)
                .await;
        }

        if !preview
            && (change.is_some()
                || new_link.is_some()
                || (!options.is_empty() && configuration.external_agent_state.options.is_empty()))
        {
            let request = RequestId::new();
            let update = SessionConfigurationChange {
                external_agent_link: new_link,
                external_agent_selection: change.is_some().then(|| ExternalAgentSelectionChange {
                    expected: expected.clone(),
                    selection: selection.clone(),
                }),
                external_agent_options: (!options.is_empty()).then(|| options.clone()),
                ..Default::default()
            };
            persist(|| {
                self.kernel()
                    .sessions
                    .configure(request, session, update.clone())
            })
            .await
            .map_err(kernel_error)?;
        }
        Ok(ExternalAgentOptions {
            selection,
            options,
            preview_commands,
        })
    }
}

fn option_offers(options: &[ExternalAgentOption], kind: &Kind, value: &str) -> bool {
    options
        .iter()
        .any(|option| &option.kind == kind && option.choices.iter().any(|choice| choice.value == value))
}

pub(super) fn config_options(response: &Value) -> Vec<ExternalAgentOption> {
    if let Some(options) = response["configOptions"].as_array() {
        let parsed = parse_options(options);
        if !parsed.is_empty() || response["modes"].is_null() {
            return parsed;
        }
    }
    let modes = &response["modes"];
    let Some(current) = option_id(&modes["currentModeId"]) else {
        return Vec::new();
    };
    let choices = modes["availableModes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|mode| parse_choice(mode, "id"))
        .take(256)
        .collect::<Vec<_>>();
    if choices.is_empty() {
        return Vec::new();
    }
    vec![ExternalAgentOption {
        id: "mode".into(),
        kind: Kind::Mode,
        name: "Mode".into(),
        description: None,
        category: Some("mode".into()),
        value_type: ExternalAgentOptionType::Select,
        legacy_mode: true,
        current_value: current.to_owned(),
        choices,
    }]
}

fn parse_options(options: &[Value]) -> Vec<ExternalAgentOption> {
    let mut parsed: Vec<ExternalAgentOption> = Vec::new();
    let mut advertised_ids = BTreeSet::new();
    for option in options {
        if parsed.len() >= 64 {
            break;
        }
        let Some(id) = option_id(&option["id"]).or_else(|| option_id(&option["configId"])) else {
            continue;
        };
        if !advertised_ids.insert(id) {
            continue;
        }
        let category = option["category"].as_str();
        let mut kind = match category {
            Some("model") => Kind::Model,
            Some("thought_level") => Kind::ThoughtLevel,
            Some("mode") => Kind::Mode,
            _ => Kind::Config(id.to_owned()),
        };
        if parsed.iter().any(|item| item.kind == kind) {
            kind = Kind::Config(id.to_owned());
        }
        let (value_type, current_value, choices) = match option["type"].as_str() {
            Some("select") => {
                let Some(current) = option_id(&option["currentValue"]) else {
                    continue;
                };
                (
                    ExternalAgentOptionType::Select,
                    current.to_owned(),
                    select_choices(option),
                )
            }
            Some("boolean") => {
                let Some(current) = option["currentValue"].as_bool() else {
                    continue;
                };
                let choices = [(false, "Off"), (true, "On")]
                    .into_iter()
                    .map(|(value, name)| ExternalAgentOptionValue {
                        value: value.to_string(),
                        name: name.into(),
                        description: None,
                    })
                    .collect();
                (
                    ExternalAgentOptionType::Boolean,
                    current.to_string(),
                    choices,
                )
            }
            _ => continue,
        };
        if !choices.is_empty() {
            parsed.push(ExternalAgentOption {
                id: id.to_owned(),
                kind,
                name: display_text(&option["name"], 256).unwrap_or_else(|| id.to_owned()),
                description: display_text(&option["description"], 512),
                category: display_text(&option["category"], 128),
                value_type,
                legacy_mode: false,
                current_value,
                choices,
            });
        }
    }
    parsed
}

fn select_choices(option: &Value) -> Vec<ExternalAgentOptionValue> {
    let mut seen = BTreeSet::new();
    option["options"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|item| {
            item["options"]
                .as_array()
                .map_or(std::slice::from_ref(item), Vec::as_slice)
        })
        .filter_map(|item| parse_choice(item, "value"))
        .filter(|item| seen.insert(item.value.clone()))
        .take(256)
        .collect()
}

fn parse_choice(item: &Value, key: &str) -> Option<ExternalAgentOptionValue> {
    let value = option_id(&item[key])?;
    Some(ExternalAgentOptionValue {
        value: value.to_owned(),
        name: display_text(&item["name"], 256).unwrap_or_else(|| value.to_owned()),
        description: display_text(&item["description"], 512),
    })
}

pub(super) fn apply_options_update(state: &mut ExternalAgentState, update: &Value) -> bool {
    if update["sessionUpdate"] == "config_option_update" {
        let Some(options) = update["configOptions"].as_array() else {
            return false;
        };
        let next = parse_options(options);
        if state.options == next {
            return false;
        }
        state.options = next;
        return true;
    }
    let Some(current) = option_id(&update["currentModeId"]) else {
        return false;
    };
    let Some(mode) = state
        .options
        .iter_mut()
        .find(|option| option.kind == Kind::Mode)
    else {
        return false;
    };
    if mode.current_value == current {
        return false;
    }
    mode.current_value = current.into();
    true
}

fn option_id(value: &Value) -> Option<&str> {
    value.as_str().filter(|value| {
        !value.trim().is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
    })
}

fn display_text(value: &Value, max: usize) -> Option<String> {
    let value = value.as_str()?.trim();
    (!value.is_empty()).then(|| value[..value.floor_char_boundary(max)].to_owned())
}
