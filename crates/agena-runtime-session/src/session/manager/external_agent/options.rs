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
            .ok_or_else(|| AppError::Config("This session has no external agent.".into()))?;
        let expected = &configuration.external_agent_selection;
        let native_linked = configuration.native_session_link().is_some();
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
        let cwd = executor.workspace_root();
        if !cwd.is_absolute() {
            return Err(AppError::Config(
                "External agent options require a project workspace.".into(),
            ));
        }
        let additional_directories = super::lifecycle::directories_for_agent(
            &executor,
            cwd,
            &configuration.external_agent_directories,
        )
        .await?;

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
        let cancel = self.shutdown.child_token();
        let mut client =
            acp::AcpClient::spawn_with_directories(agent, cwd, &additional_directories)?;
        let outcome = async {
            client.prepare_provider_routes(&cancel).await?;
            if preview || (change.is_none() && selection.is_empty()) {
                let previous = configuration.external_agent_resume.as_ref()
                    .filter(|prior| prior.session == session && prior.cwd == cwd)
                    .map(|prior| prior.native_id.as_str());
                match client.session_options(cwd, previous, &additional_directories, &cancel).await {
                    Ok(Some(response)) => {
                        let commands = if !native_linked && client.uses_v2() {
                            super::state::available_commands(&response["availableCommands"])
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        return Ok((config_options(&response), commands));
                    }
                    Ok(None) => {}
                    Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                    Err(error) if preview => return Err(error),
                    Err(error) => tracing::debug!(%error, "could not inspect ACP options without opening a session"),
                }
            }
            if preview {
                return Ok((Vec::new(), Vec::new()));
            }
            let mut opened = client.start_probe(cwd, additional_directories, &cancel).await?;
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
                    client.set_option(&mut opened, &kind, &value, &cancel).await?;
                }
                Ok::<_, AppError>(())
            }
            .await;
            client.close_probe(&opened, &cancel).await;
            result?;
            Ok::<_, AppError>((opened.options, Vec::new()))
        }
        .await;
        client.close().await;
        let (options, preview_commands) = outcome?;

        if change.is_some() {
            let request = RequestId::new();
            let update = SessionConfigurationChange {
                external_agent_selection: Some(ExternalAgentSelectionChange {
                    expected: expected.clone(),
                    selection: selection.clone(),
                }),
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

pub(super) fn config_options(response: &Value) -> Vec<ExternalAgentOption> {
    if let Some(options) = response["configOptions"].as_array() {
        return parse_options(options);
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
