use std::collections::BTreeMap;

use crate::error::AppError;

use super::protocol_ids::{ModelToolCallId, ProviderItemId, ProviderStreamKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolStreamInputKind {
    Start,
    Delta,
    Finish,
}

#[derive(Debug, Clone)]
pub(crate) struct ToolStreamInput {
    pub(crate) kind: ToolStreamInputKind,
    pub(crate) stream_key_candidates: Vec<ProviderStreamKey>,
    pub(crate) provider_item_id: Option<ProviderItemId>,
    pub(crate) model_call_id: Option<ModelToolCallId>,
    pub(crate) name: Option<String>,
    pub(crate) arguments: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolStreamUpdate {
    Registered {
        stream_key: String,
        id: Option<String>,
        name: Option<String>,
    },
    ArgumentsDelta {
        stream_key: String,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    ArgumentsSnapshot {
        stream_key: String,
        id: Option<String>,
        name: Option<String>,
        arguments_json: String,
    },
}

#[derive(Debug, Default)]
pub(crate) struct ToolStreamAccumulator {
    aliases: BTreeMap<String, ProviderStreamKey>,
    pending: BTreeMap<ProviderStreamKey, ToolStreamState>,
}

#[derive(Debug, Default)]
struct ToolStreamState {
    provider_item_id: Option<ProviderItemId>,
    model_call_id: Option<ModelToolCallId>,
    name: Option<String>,
    arguments: String,
    registered: bool,
}

impl ToolStreamAccumulator {
    pub(crate) fn ingest(
        &mut self,
        provider_id: &str,
        input: ToolStreamInput,
    ) -> Result<Vec<ToolStreamUpdate>, AppError> {
        let stream_key = self.resolve_stream_key(provider_id, &input.stream_key_candidates)?;
        let was_new = !self.pending.contains_key(&stream_key);
        let state = self.pending.entry(stream_key.clone()).or_default();

        let previous_model_call_id = state.model_call_id.clone();
        let previous_name = state.name.clone();
        if let Some(provider_item_id) = input.provider_item_id {
            state.provider_item_id = Some(provider_item_id);
        }
        if let Some(model_call_id) = input.model_call_id {
            state.model_call_id = Some(model_call_id);
        }
        if let Some(name) = input.name.filter(|value| !value.trim().is_empty()) {
            state.name = Some(name);
        }

        let mut updates = Vec::new();
        if matches!(
            input.kind,
            ToolStreamInputKind::Start | ToolStreamInputKind::Finish
        ) && (was_new || !state.registered)
        {
            state.registered = true;
            updates.push(state.registered_update(stream_key.as_str()));
        }

        match input.kind {
            ToolStreamInputKind::Delta => {
                if let Some(arguments_delta) = input.arguments.filter(|value| !value.is_empty()) {
                    state.arguments.push_str(arguments_delta.as_str());
                    if !state.registered {
                        state.registered = true;
                    }
                    updates
                        .push(state.arguments_delta_update(stream_key.as_str(), arguments_delta));
                }
            }
            ToolStreamInputKind::Start | ToolStreamInputKind::Finish => {
                if let Some(arguments_snapshot) = input.arguments.filter(|value| !value.is_empty())
                {
                    if let Some(arguments_delta) =
                        snapshot_delta(&mut state.arguments, arguments_snapshot.as_str())
                    {
                        if !state.registered {
                            state.registered = true;
                        }
                        updates.push(match arguments_delta {
                            SnapshotEffect::Append(delta) => {
                                state.arguments_delta_update(stream_key.as_str(), delta)
                            }
                            SnapshotEffect::Replace(snapshot) => {
                                state.arguments_snapshot_update(stream_key.as_str(), snapshot)
                            }
                        });
                    }
                }
            }
        }

        let metadata_changed =
            previous_model_call_id != state.model_call_id || previous_name != state.name;
        if updates.is_empty() && state.registered && metadata_changed {
            updates.push(state.registered_update(stream_key.as_str()));
        }

        Ok(updates)
    }

    fn resolve_stream_key(
        &mut self,
        provider_id: &str,
        candidates: &[ProviderStreamKey],
    ) -> Result<ProviderStreamKey, AppError> {
        if candidates.is_empty() {
            return Err(AppError::Provider(format!(
                "{provider_id} returned tool event without stream key candidates"
            )));
        }

        let key = candidates
            .iter()
            .find_map(|candidate| self.aliases.get(candidate.as_str()).cloned())
            .or_else(|| {
                candidates
                    .iter()
                    .find(|candidate| self.pending.contains_key(*candidate))
                    .cloned()
            })
            .unwrap_or_else(|| candidates[0].clone());

        for candidate in candidates {
            self.aliases
                .insert(candidate.as_str().to_owned(), key.clone());
        }

        Ok(key)
    }
}

impl ToolStreamState {
    fn registered_update(&self, stream_key: &str) -> ToolStreamUpdate {
        ToolStreamUpdate::Registered {
            stream_key: stream_key.to_owned(),
            id: self.model_call_id.as_ref().map(|id| id.as_str().to_owned()),
            name: self.name.clone(),
        }
    }

    fn arguments_delta_update(
        &self,
        stream_key: &str,
        arguments_delta: String,
    ) -> ToolStreamUpdate {
        ToolStreamUpdate::ArgumentsDelta {
            stream_key: stream_key.to_owned(),
            id: self.model_call_id.as_ref().map(|id| id.as_str().to_owned()),
            name: self.name.clone(),
            arguments_delta,
        }
    }

    fn arguments_snapshot_update(
        &self,
        stream_key: &str,
        arguments_json: String,
    ) -> ToolStreamUpdate {
        ToolStreamUpdate::ArgumentsSnapshot {
            stream_key: stream_key.to_owned(),
            id: self.model_call_id.as_ref().map(|id| id.as_str().to_owned()),
            name: self.name.clone(),
            arguments_json,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SnapshotEffect {
    Append(String),
    Replace(String),
}

fn snapshot_delta(current: &mut String, snapshot: &str) -> Option<SnapshotEffect> {
    if snapshot.starts_with(current.as_str()) {
        let delta = snapshot[current.len()..].to_owned();
        if delta.is_empty() {
            return None;
        }
        current.push_str(delta.as_str());
        return Some(SnapshotEffect::Append(delta));
    }

    current.clear();
    current.push_str(snapshot);
    Some(SnapshotEffect::Replace(snapshot.to_owned()))
}
