use std::collections::BTreeMap;

use crate::{ModelToolCallId, ProviderItemId, ProviderStreamKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Kind of a tool stream input event.
pub enum ToolStreamInputKind {
    Start,
    Delta,
    Finish,
}

#[derive(Debug, Clone)]
/// Input event for provider tool streaming.
pub struct ToolStreamInput {
    pub kind: ToolStreamInputKind,
    pub stream_key_candidates: Vec<ProviderStreamKey>,
    pub provider_item_id: Option<ProviderItemId>,
    pub model_call_id: Option<ModelToolCallId>,
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Update emitted by the tool stream accumulator.
pub enum ToolStreamUpdate {
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

#[derive(Debug)]
/// Accumulates provider tool stream events into tool calls.
pub struct ToolStreamAccumulator {
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

#[derive(Debug, thiserror::Error)]
/// Error from provider tool streaming.
pub enum ToolStreamError {
    #[error("{provider_id} returned tool event without stream key candidates")]
    MissingStreamKey { provider_id: String },
}

impl ToolStreamAccumulator {
    pub fn new() -> Self {
        Self {
            aliases: BTreeMap::new(),
            pending: BTreeMap::new(),
        }
    }

    pub fn ingest(
        &mut self,
        provider_id: &str,
        input: ToolStreamInput,
    ) -> Result<Vec<ToolStreamUpdate>, ToolStreamError> {
        let stream_key = self.resolve_stream_key(provider_id, &input)?;
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
            updates.push(state.registered_update(stream_key.as_ref()));
        }

        match input.kind {
            ToolStreamInputKind::Delta => {
                if let Some(arguments_delta) = input.arguments.filter(|value| !value.is_empty()) {
                    state.arguments.push_str(arguments_delta.as_str());
                    if !state.registered {
                        state.registered = true;
                    }
                    updates
                        .push(state.arguments_delta_update(stream_key.as_ref(), arguments_delta));
                }
            }
            ToolStreamInputKind::Start | ToolStreamInputKind::Finish => {
                if let Some(arguments_snapshot) = input.arguments.filter(|value| !value.is_empty())
                    && let Some(arguments_delta) =
                        snapshot_delta(&mut state.arguments, arguments_snapshot.as_str())
                {
                    if !state.registered {
                        state.registered = true;
                    }
                    updates.push(match arguments_delta {
                        SnapshotEffect::Append(delta) => {
                            state.arguments_delta_update(stream_key.as_ref(), delta)
                        }
                        SnapshotEffect::Replace(snapshot) => {
                            state.arguments_snapshot_update(stream_key.as_ref(), snapshot)
                        }
                    });
                }
            }
        }

        let metadata_changed =
            previous_model_call_id != state.model_call_id || previous_name != state.name;
        if updates.is_empty() && state.registered && metadata_changed {
            updates.push(state.registered_update(stream_key.as_ref()));
        }

        Ok(updates)
    }

    fn resolve_stream_key(
        &mut self,
        provider_id: &str,
        input: &ToolStreamInput,
    ) -> Result<ProviderStreamKey, ToolStreamError> {
        let candidates = input.stream_key_candidates.as_slice();
        if candidates.is_empty() {
            return Err(ToolStreamError::MissingStreamKey {
                provider_id: provider_id.to_owned(),
            });
        }

        // A model call id is authoritative. Do not let a reused positional
        // index alias a different call id into an earlier call; OpenAI-style
        // providers may legitimately reuse an index across separate calls.
        let authoritative = input.model_call_id.as_ref().and_then(|model_call_id| {
            candidates.iter().find(|candidate| {
                candidate
                    .as_ref()
                    .split_once(':')
                    .is_some_and(|(_, value)| value == model_call_id.as_ref())
            })
        });
        let provider_item = input
            .provider_item_id
            .as_ref()
            .and_then(|provider_item_id| {
                let expected = format!("item:{provider_item_id}");
                candidates
                    .iter()
                    .find(|candidate| candidate.as_ref() == expected)
            });
        let resolve_existing = |candidate: &ProviderStreamKey| {
            self.aliases.get(candidate.as_ref()).cloned().or_else(|| {
                self.pending
                    .contains_key(candidate)
                    .then(|| candidate.clone())
            })
        };
        let key = authoritative
            .and_then(&resolve_existing)
            // A delta can arrive before output_item.added and therefore before
            // call_id is known. When the final item supplies call_id, promote
            // the state already keyed by the same provider item rather than
            // creating a second call. Positional indices are intentionally not
            // used for this promotion because providers may reuse them.
            .or_else(|| provider_item.and_then(resolve_existing))
            .or_else(|| {
                // A `call_id`-only Finish/Start event (no item id) must not
                // open a second accumulator key when the same logical call
                // already has a live stream reachable through another candidate
                // (for example a positional index that a prior item-based delta
                // aliased). Adopt that live stream only when it carries no
                // conflicting call id: distinct call ids must stay independent
                // even when an adapter reuses a positional index.
                let authoritative_id = authoritative.as_ref().and_then(|candidate| {
                    candidate
                        .as_ref()
                        .split_once(':')
                        .map(|(_, value)| value.to_owned())
                });
                candidates.iter().find_map(|candidate| {
                    let existing = resolve_existing(candidate)?;
                    let conflicts = self.pending.get(&existing).is_some_and(|state| {
                        authoritative_id.as_deref().is_some_and(|id| {
                            state
                                .model_call_id
                                .as_ref()
                                .is_some_and(|existing_id| existing_id.as_ref() != id)
                        })
                    });
                    if conflicts {
                        None
                    } else {
                        Some(existing)
                    }
                })
            })
            .or_else(|| authoritative.cloned())
            .or_else(|| {
                candidates
                    .iter()
                    .find_map(|candidate| self.aliases.get(candidate.as_ref()).cloned())
            })
            .or_else(|| {
                candidates
                    .iter()
                    .find(|candidate| self.pending.contains_key(*candidate))
                    .cloned()
            })
            .unwrap_or_else(|| candidates[0].clone());

        for candidate in candidates {
            self.aliases
                .insert(candidate.as_ref().to_owned(), key.clone());
        }

        Ok(key)
    }
}

impl Default for ToolStreamAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolStreamState {
    fn registered_update(&self, stream_key: &str) -> ToolStreamUpdate {
        ToolStreamUpdate::Registered {
            stream_key: stream_key.to_owned(),
            id: self.model_call_id.as_ref().map(|id| id.as_ref().to_owned()),
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
            id: self.model_call_id.as_ref().map(|id| id.as_ref().to_owned()),
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
            id: self.model_call_id.as_ref().map(|id| id.as_ref().to_owned()),
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
    let snapshot_trimmed = snapshot.trim();
    if !current.trim().is_empty() && (snapshot_trimmed.is_empty() || snapshot_trimmed == "{}") {
        return None;
    }
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

#[cfg(test)]
mod tests {
    use super::{ToolStreamAccumulator, ToolStreamInput, ToolStreamInputKind, ToolStreamUpdate};

    fn input(
        kind: ToolStreamInputKind,
        keys: &[&str],
        call_id: Option<&str>,
        arguments: Option<&str>,
    ) -> ToolStreamInput {
        ToolStreamInput {
            kind,
            stream_key_candidates: keys
                .iter()
                .map(|key| key.parse().expect("non-empty stream key"))
                .collect(),
            provider_item_id: None,
            model_call_id: call_id.map(|id| id.parse().expect("non-empty call id")),
            name: Some("tools_call".to_string()),
            arguments: arguments.map(ToOwned::to_owned),
        }
    }

    fn input_with_item(
        kind: ToolStreamInputKind,
        keys: &[&str],
        item_id: &str,
        call_id: Option<&str>,
        arguments: Option<&str>,
    ) -> ToolStreamInput {
        let mut input = input(kind, keys, call_id, arguments);
        input.provider_item_id = Some(item_id.parse().expect("non-empty item id"));
        input
    }

    fn update_stream_key(update: &ToolStreamUpdate) -> &str {
        match update {
            ToolStreamUpdate::Registered { stream_key, .. }
            | ToolStreamUpdate::ArgumentsDelta { stream_key, .. }
            | ToolStreamUpdate::ArgumentsSnapshot { stream_key, .. } => stream_key.as_str(),
        }
    }

    fn update_id(update: &ToolStreamUpdate) -> Option<&str> {
        match update {
            ToolStreamUpdate::Registered { id, .. }
            | ToolStreamUpdate::ArgumentsDelta { id, .. }
            | ToolStreamUpdate::ArgumentsSnapshot { id, .. } => id.as_deref(),
        }
    }

    #[test]
    fn aliases_changing_indices_by_the_shared_call_id() {
        let mut accumulator = ToolStreamAccumulator::new();

        let first = accumulator
            .ingest(
                "cline",
                input(
                    ToolStreamInputKind::Delta,
                    &["id:call_shared", "idx:0"],
                    Some("call_shared"),
                    Some(r#"{"tool":"skills."#),
                ),
            )
            .expect("first tool chunk");
        assert_eq!(
            first,
            vec![ToolStreamUpdate::ArgumentsDelta {
                stream_key: "id:call_shared".to_string(),
                id: Some("call_shared".to_string()),
                name: Some("tools_call".to_string()),
                arguments_delta: r#"{"tool":"skills."#.to_string(),
            }]
        );

        let continuation = accumulator
            .ingest(
                "cline",
                input(
                    ToolStreamInputKind::Delta,
                    &["id:call_shared", "idx:6"],
                    Some("call_shared"),
                    Some(r#"list","input":{}}"#),
                ),
            )
            .expect("continued tool chunk");
        assert_eq!(
            continuation,
            vec![ToolStreamUpdate::ArgumentsDelta {
                stream_key: "id:call_shared".to_string(),
                id: Some("call_shared".to_string()),
                name: Some("tools_call".to_string()),
                arguments_delta: r#"list","input":{}}"#.to_string(),
            }]
        );
    }

    #[test]
    fn preserves_distinct_call_ids_with_identical_arguments() {
        let mut accumulator = ToolStreamAccumulator::new();
        let arguments = r#"{"tool":"skills.list","input":{}}"#;

        let first = accumulator
            .ingest(
                "cline",
                input(
                    ToolStreamInputKind::Delta,
                    &["id:call_one", "idx:0"],
                    Some("call_one"),
                    Some(arguments),
                ),
            )
            .expect("first call");
        let second = accumulator
            .ingest(
                "cline",
                input(
                    ToolStreamInputKind::Delta,
                    &["id:call_two", "idx:0"],
                    Some("call_two"),
                    Some(arguments),
                ),
            )
            .expect("second call");

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        let ToolStreamUpdate::ArgumentsDelta {
            stream_key: first_key,
            ..
        } = &first[0]
        else {
            panic!("first update should be an argument delta");
        };
        let ToolStreamUpdate::ArgumentsDelta {
            stream_key: second_key,
            ..
        } = &second[0]
        else {
            panic!("second update should be an argument delta");
        };
        assert_ne!(first_key, second_key);
    }

    #[test]
    fn late_call_id_promotes_the_existing_provider_item_stream() {
        let mut accumulator = ToolStreamAccumulator::new();

        let first = accumulator
            .ingest(
                "openai",
                input_with_item(
                    ToolStreamInputKind::Delta,
                    &["item:fc_1", "idx:0"],
                    "fc_1",
                    None,
                    Some(r#"{"limit":"#),
                ),
            )
            .expect("idless argument delta");
        assert_eq!(
            first,
            vec![ToolStreamUpdate::ArgumentsDelta {
                stream_key: "item:fc_1".to_string(),
                id: None,
                name: Some("tools_call".to_string()),
                arguments_delta: r#"{"limit":"#.to_string(),
            }]
        );

        let finished = accumulator
            .ingest(
                "openai",
                input_with_item(
                    ToolStreamInputKind::Finish,
                    &["item:fc_1", "idx:0", "call:call_1"],
                    "fc_1",
                    Some("call_1"),
                    Some(r#"{"limit":100}"#),
                ),
            )
            .expect("completed output item");
        assert_eq!(
            finished,
            vec![ToolStreamUpdate::ArgumentsDelta {
                stream_key: "item:fc_1".to_string(),
                id: Some("call_1".to_string()),
                name: Some("tools_call".to_string()),
                arguments_delta: "100}".to_string(),
            }]
        );
    }

    #[test]
    fn degenerate_snapshot_does_not_erase_accumulated_arguments() {
        let mut accumulator = ToolStreamAccumulator::new();
        let arguments = r#"{"tool":"fs.read","input":{"file_path":"README.md"}}"#;
        accumulator
            .ingest(
                "openai",
                input(
                    ToolStreamInputKind::Delta,
                    &["call:call_1", "idx:0"],
                    Some("call_1"),
                    Some(arguments),
                ),
            )
            .expect("complete argument delta");

        let empty_trailer = accumulator
            .ingest(
                "openai",
                input(
                    ToolStreamInputKind::Finish,
                    &["call:call_1", "idx:0"],
                    Some("call_1"),
                    Some("{}"),
                ),
            )
            .expect("degenerate finish snapshot");
        assert!(
            empty_trailer.is_empty(),
            "an empty object trailer is not an argument replacement"
        );

        let repeated_full_snapshot = accumulator
            .ingest(
                "openai",
                input(
                    ToolStreamInputKind::Finish,
                    &["call:call_1", "idx:0"],
                    Some("call_1"),
                    Some(arguments),
                ),
            )
            .expect("repeat the authoritative snapshot");
        assert!(
            repeated_full_snapshot.is_empty(),
            "the authoritative arguments remained accumulated"
        );
    }

    #[test]
    fn call_id_only_finish_joins_the_existing_item_stream() {
        let mut accumulator = ToolStreamAccumulator::new();

        // A `function_call_arguments.delta` event carries only the item id and
        // lands on `item:fc_abc`. The later `done` event carries `call_id` but
        // (on a compatible gateway) no item id; it must join the same live
        // stream instead of opening a second key for one logical call.
        let delta = accumulator
            .ingest(
                "openai",
                input_with_item(
                    ToolStreamInputKind::Delta,
                    &["item:fc_abc", "idx:0"],
                    "fc_abc",
                    None,
                    Some(r#"{"tool":"fs.grep","input":{"#),
                ),
            )
            .expect("idless argument delta");
        assert_eq!(
            delta,
            vec![ToolStreamUpdate::ArgumentsDelta {
                stream_key: "item:fc_abc".to_string(),
                id: None,
                name: Some("tools_call".to_string()),
                arguments_delta: r#"{"tool":"fs.grep","input":{"#.to_string(),
            }]
        );

        // Done carries `call:call_01` and a complete arguments snapshot but no
        // item id. It must resolve to the existing `item:fc_abc` stream instead
        // of opening a second key. The delta is appended because the snapshot
        // continues the already-accumulated arguments.
        let finished = accumulator
            .ingest(
                "openai",
                input(
                    ToolStreamInputKind::Finish,
                    &["call:call_01", "idx:0"],
                    Some("call_01"),
                    Some(r#"{"tool":"fs.grep","input":{"path":"x"}}"#),
                ),
            )
            .expect("call-id-only done event");
        let finished_key = update_stream_key(&finished[0]);
        assert_eq!(
            finished_key, "item:fc_abc",
            "call-id-only done must join the existing item stream"
        );
        let finished_id = update_id(&finished[0]);
        assert_eq!(finished_id, Some("call_01"));

        // No second key was opened: a further delta still lands on the item key.
        let more = accumulator
            .ingest(
                "openai",
                input_with_item(
                    ToolStreamInputKind::Delta,
                    &["item:fc_abc", "idx:0"],
                    "fc_abc",
                    None,
                    Some(" }"),
                ),
            )
            .expect("continuation delta");
        assert_eq!(
            update_stream_key(&more[0]),
            "item:fc_abc",
            "the stream must remain keyed by the item id after the done event"
        );
    }

    #[test]
    fn distinct_call_ids_stay_independent_when_a_call_id_only_finish_reuses_an_index() {
        let mut accumulator = ToolStreamAccumulator::new();

        // Two parallel calls share the same positional index. Their deltas are
        // keyed by distinct call ids, so a `call_id`-only Finish for a second
        // call must not be folded into the first call's live stream.
        let first = accumulator
            .ingest(
                "openai",
                input(
                    ToolStreamInputKind::Delta,
                    &["call:call_one", "idx:0"],
                    Some("call_one"),
                    Some(r#"{"tool":"fs.read","input":{"file_path":"a"}}"#),
                ),
            )
            .expect("first call delta");
        let first_key = update_stream_key(&first[0]);

        let second = accumulator
            .ingest(
                "openai",
                input(
                    ToolStreamInputKind::Finish,
                    &["call:call_two", "idx:0"],
                    Some("call_two"),
                    Some(r#"{"tool":"fs.grep","input":{"path":"b"}}"#),
                ),
            )
            .expect("second call finish");
        let second_key = update_stream_key(&second[0]);

        assert_ne!(
            first_key, second_key,
            "a reused positional index must not merge distinct call ids"
        );
        assert_eq!(
            second_key, "call:call_two",
            "the second call must keep its authoritative key"
        );
    }

    #[test]
    fn two_parallel_responses_calls_with_distinct_indices_stay_separate() {
        // Reproduce the cpa gateway's Responses event shape for two parallel
        // tools_call invocations: each call has an output_item.added (with
        // call_id + item_id), then item-based argument deltas, then a done
        // (with call_id + item_id + full arguments). The two calls use
        // distinct output indices. Each logical call must stay on one key and
        // accumulate its own full arguments.
        let mut accumulator = ToolStreamAccumulator::new();

        // Call A: fs.grep (output_index 0)
        let added_a = accumulator
            .ingest(
                "openai",
                input_with_item(
                    ToolStreamInputKind::Start,
                    &["item:fc_1", "idx:0", "call:call_00"],
                    "fc_1",
                    Some("call_00"),
                    None,
                ),
            )
            .expect("call A added");
        assert_eq!(update_stream_key(&added_a[0]), "call:call_00");

        // Call B: shell.run (output_index 1)
        let added_b = accumulator
            .ingest(
                "openai",
                input_with_item(
                    ToolStreamInputKind::Start,
                    &["item:fc_2", "idx:1", "call:call_01"],
                    "fc_2",
                    Some("call_01"),
                    None,
                ),
            )
            .expect("call B added");
        assert_eq!(update_stream_key(&added_b[0]), "call:call_01");

        // Call A argument deltas (item-only, no call_id)
        accumulator
            .ingest(
                "openai",
                input_with_item(
                    ToolStreamInputKind::Delta,
                    &["item:fc_1", "idx:0"],
                    "fc_1",
                    None,
                    Some(r#"{"tool":"fs.grep","input":{"pattern":"fn "#),
                ),
            )
            .expect("call A delta 1");
        accumulator
            .ingest(
                "openai",
                input_with_item(
                    ToolStreamInputKind::Delta,
                    &["item:fc_1", "idx:0"],
                    "fc_1",
                    None,
                    Some(r#"turn_id"}}"#),
                ),
            )
            .expect("call A delta 2");

        // Call B argument deltas (item-only, no call_id)
        accumulator
            .ingest(
                "openai",
                input_with_item(
                    ToolStreamInputKind::Delta,
                    &["item:fc_2", "idx:1"],
                    "fc_2",
                    None,
                    Some(r#"{"tool":"shell.run","input":{"command":"grep"}"#),
                ),
            )
            .expect("call B delta 1");

        // Call A done (call_id + a continuation that differs from the
        // accumulated deltas so a delta is emitted on call A's key)
        let done_a = accumulator
            .ingest(
                "openai",
                input_with_item(
                    ToolStreamInputKind::Finish,
                    &["item:fc_1", "idx:0", "call:call_00"],
                    "fc_1",
                    Some("call_00"),
                    Some(r#"{"tool":"fs.grep","input":{"pattern":"fn turn_id","extra":1}}"#),
                ),
            )
            .expect("call A done");
        assert!(
            !done_a.is_empty()
                && done_a
                    .iter()
                    .all(|update| update_stream_key(update) == "call:call_00"),
            "call A done must stay on call A's key: {done_a:?}"
        );

        // Call B done (call_id + continuation)
        let done_b = accumulator
            .ingest(
                "openai",
                input_with_item(
                    ToolStreamInputKind::Finish,
                    &["item:fc_2", "idx:1", "call:call_01"],
                    "fc_2",
                    Some("call_01"),
                    Some(r#"{"tool":"shell.run","input":{"command":"grep -r","extra":2}}"#),
                ),
            )
            .expect("call B done");
        assert!(
            !done_b.is_empty()
                && done_b
                    .iter()
                    .all(|update| update_stream_key(update) == "call:call_01"),
            "call B done must stay on call B's key: {done_b:?}"
        );
    }
}
