use super::EventKind;

/// Give a copied/imported event stream fresh domain identities.
///
/// Message/part integers are storage addresses and are remapped separately.
/// These UUIDs are semantic identities and must also be fresh: sharing an
/// Execution, Turn, AssistantReply, Activity, or text segment across two sessions
/// would make ownership ambiguous and violate the canonical transcript keys.
pub(crate) fn rewrite_copied_domain_ids(items: &mut [EventKind]) {
    #[derive(Default)]
    struct Maps {
        executions: std::collections::HashMap<agena_domain::ExecutionId, agena_domain::ExecutionId>,
        runs: std::collections::HashMap<agena_domain::RunId, agena_domain::RunId>,
        turns: std::collections::HashMap<agena_domain::TurnId, agena_domain::TurnId>,
        replies: std::collections::HashMap<
            agena_domain::AssistantReplyId,
            agena_domain::AssistantReplyId,
        >,
        activities: std::collections::HashMap<agena_domain::ActivityId, agena_domain::ActivityId>,
        segments:
            std::collections::HashMap<agena_domain::TextSegmentId, agena_domain::TextSegmentId>,
    }

    fn execution(maps: &mut Maps, id: agena_domain::ExecutionId) -> agena_domain::ExecutionId {
        *maps.executions.entry(id).or_default()
    }
    fn run(maps: &mut Maps, id: agena_domain::RunId) -> agena_domain::RunId {
        *maps.runs.entry(id).or_default()
    }
    fn turn(maps: &mut Maps, id: agena_domain::TurnId) -> agena_domain::TurnId {
        *maps.turns.entry(id).or_default()
    }
    fn reply(
        maps: &mut Maps,
        id: agena_domain::AssistantReplyId,
    ) -> agena_domain::AssistantReplyId {
        *maps.replies.entry(id).or_default()
    }
    fn activity(maps: &mut Maps, id: agena_domain::ActivityId) -> agena_domain::ActivityId {
        *maps.activities.entry(id).or_default()
    }
    fn segment(maps: &mut Maps, id: agena_domain::TextSegmentId) -> agena_domain::TextSegmentId {
        *maps.segments.entry(id).or_default()
    }
    fn part(maps: &mut Maps, part: &mut crate::message::MessagePart) {
        if let Some(id) = part.activity_id {
            part.activity_id = Some(activity(maps, id));
        }
        if let Some(id) = part.segment_id {
            part.segment_id = Some(segment(maps, id));
        }
    }

    let mut maps = Maps::default();
    for item in items {
        match item {
            EventKind::ExecutionStarted(value) => {
                value.execution_id = execution(&mut maps, value.execution_id);
                value.turn_id = turn(&mut maps, value.turn_id);
                value.reply_id = reply(&mut maps, value.reply_id);
            }
            EventKind::ExecutionFinished(value) => {
                value.execution_id = execution(&mut maps, value.execution_id);
                value.reply_id = reply(&mut maps, value.reply_id);
            }
            EventKind::CompactionCompleted(value) => {
                value.execution_id = execution(&mut maps, value.execution_id);
                value.activity_id = activity(&mut maps, value.activity_id);
            }
            EventKind::MessagePartCheckpointed(value) => {
                value.execution_id = value.execution_id.map(|id| execution(&mut maps, id));
                value.run_id = value.run_id.map(|id| run(&mut maps, id));
                value.turn_id = value.turn_id.map(|id| turn(&mut maps, id));
                value.reply_id = value.reply_id.map(|id| reply(&mut maps, id));
                part(&mut maps, &mut value.part);
            }
            EventKind::RunStarted(value) => {
                value.execution_id = execution(&mut maps, value.execution_id);
                value.run_id = run(&mut maps, value.run_id);
            }
            EventKind::RunCompleted(value) => value.run_id = run(&mut maps, value.run_id),
            EventKind::RunAborted(value) => value.run_id = run(&mut maps, value.run_id),
            EventKind::UserMessageAppended(value) => {
                value.execution_id = execution(&mut maps, value.execution_id);
                value.run_id = run(&mut maps, value.run_id);
                for value in &mut value.parts {
                    part(&mut maps, value);
                }
            }
            EventKind::AssistantMessageFinished(value) => {
                value.execution_id = execution(&mut maps, value.execution_id);
                value.run_id = run(&mut maps, value.run_id);
                for value in &mut value.parts {
                    part(&mut maps, value);
                }
            }
            EventKind::ToolCallIssued(value) => value.run_id = run(&mut maps, value.run_id),
            EventKind::ToolCallCompleted(value) => {
                value.run_id = run(&mut maps, value.run_id);
            }
            EventKind::SubtaskStatusChanged(_)
            | EventKind::StreamError(_)
            | EventKind::ProviderRetry(_)
            | EventKind::ProviderRetryResolved(_)
            | EventKind::TranscriptPartUpserted(_)
            | EventKind::CommandBegin(_)
            | EventKind::CommandOutputDelta(_)
            | EventKind::CommandEnd(_)
            | EventKind::PermissionRequested(_)
            | EventKind::UserInputRequested(_)
            | EventKind::PermissionReplied(_)
            | EventKind::PermissionRuleCreated(_)
            | EventKind::PermissionRuleUpdated(_)
            | EventKind::PermissionRuleRevoked(_)
            | EventKind::ToolPolicyDenied(_)
            | EventKind::ToolUserDeclined(_)
            | EventKind::BackgroundActivityChanged(_)
            | EventKind::PluginEvent(_)
            | EventKind::PluginToolRegistryChanged(_)
            | EventKind::ActivityV2(_) => {}
        }
    }
}

pub(crate) fn event_targets_message(kind: &EventKind, message_id: i64) -> bool {
    match kind {
        EventKind::MessagePartCheckpointed(payload) => payload.message_id == message_id,
        EventKind::UserMessageAppended(payload) => payload.message_id.raw() == message_id,
        EventKind::AssistantMessageFinished(payload) => payload.message_id.raw() == message_id,
        EventKind::ToolCallIssued(payload) => payload.message_id.raw() == message_id,
        EventKind::ToolCallCompleted(payload) => payload.message_id.raw() == message_id,
        EventKind::CompactionCompleted(_) => false,
        _ => false,
    }
}

pub(crate) fn event_run_id_for_message(
    kind: &EventKind,
    message_id: i64,
) -> Option<agena_domain::RunId> {
    match kind {
        EventKind::MessagePartCheckpointed(payload) if payload.message_id == message_id => {
            payload.run_id
        }
        EventKind::UserMessageAppended(payload) if payload.message_id.raw() == message_id => {
            Some(payload.run_id)
        }
        EventKind::AssistantMessageFinished(payload) if payload.message_id.raw() == message_id => {
            Some(payload.run_id)
        }
        EventKind::ToolCallIssued(payload) if payload.message_id.raw() == message_id => {
            Some(payload.run_id)
        }
        EventKind::ToolCallCompleted(payload) if payload.message_id.raw() == message_id => {
            Some(payload.run_id)
        }
        _ => None,
    }
}

pub(crate) fn event_execution_id_for_message(
    kind: &EventKind,
    message_id: i64,
) -> Option<agena_domain::ExecutionId> {
    match kind {
        EventKind::MessagePartCheckpointed(payload) if payload.message_id == message_id => {
            payload.execution_id
        }
        EventKind::UserMessageAppended(payload) if payload.message_id.raw() == message_id => {
            Some(payload.execution_id)
        }
        EventKind::AssistantMessageFinished(payload) if payload.message_id.raw() == message_id => {
            Some(payload.execution_id)
        }
        _ => None,
    }
}

/// Visit every `message_id` carried by the persistent variants of `kind`.
/// Stays in sync with [`rewrite_event_message_ids`] — anything visited there
/// must be visited here too, otherwise import will under-reserve and
/// imported ids will collide with later live ids.
pub(crate) fn visit_event_message_ids(kind: &EventKind, mut visit: impl FnMut(i64)) {
    match kind {
        EventKind::UserMessageAppended(p) => {
            visit(p.message_id.raw());
            visit_message_metadata_ids(&p.metadata, &mut visit);
            for part in &p.parts {
                visit(part.message_id);
            }
        }
        EventKind::AssistantMessageFinished(p) => {
            visit(p.message_id.raw());
            visit_message_metadata_ids(&p.metadata, &mut visit);
            for part in &p.parts {
                visit(part.message_id);
            }
        }
        EventKind::ToolCallIssued(p) => visit(p.message_id.raw()),
        EventKind::ToolCallCompleted(p) => {
            visit(p.message_id.raw());
        }
        EventKind::CompactionCompleted(p) => {
            visit(p.activity.compacted_through_message_id);
        }
        EventKind::MessagePartCheckpointed(p) => {
            visit(p.message_id);
            visit_message_metadata_ids(&p.message_metadata, &mut visit);
            visit(p.part.message_id);
        }
        // Non-persistent / unaffected variants:
        EventKind::ExecutionStarted(_)
        | EventKind::ExecutionFinished(_)
        | EventKind::SubtaskStatusChanged(_)
        | EventKind::StreamError(_)
        | EventKind::ProviderRetry(_)
        | EventKind::ProviderRetryResolved(_)
        | EventKind::TranscriptPartUpserted(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::UserInputRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::ToolPolicyDenied(_)
        | EventKind::ToolUserDeclined(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::BackgroundActivityChanged(_)
        | EventKind::PluginEvent(_)
        | EventKind::PluginToolRegistryChanged(_)
        | EventKind::ActivityV2(_) => {}
    }
}

pub(crate) fn visit_message_metadata_ids(
    metadata: &crate::message::MessageMetadata,
    mut visit: impl FnMut(i64),
) {
    if let Some(turn_id) = metadata.model_turn_id {
        visit(turn_id);
    }
    if let Some(parent_message_id) = metadata.parent_message_id {
        visit(parent_message_id);
    }
}

pub(crate) fn visit_event_part_ids(kind: &EventKind, mut visit: impl FnMut(i64)) {
    match kind {
        EventKind::UserMessageAppended(p) => {
            for part in &p.parts {
                visit(part.id);
            }
        }
        EventKind::AssistantMessageFinished(p) => {
            for part in &p.parts {
                visit(part.id);
            }
        }
        EventKind::MessagePartCheckpointed(p) => {
            visit(p.part.id);
        }
        EventKind::CompactionCompleted(_) => {}
        EventKind::ExecutionStarted(_)
        | EventKind::ExecutionFinished(_)
        | EventKind::SubtaskStatusChanged(_)
        | EventKind::StreamError(_)
        | EventKind::ProviderRetry(_)
        | EventKind::ProviderRetryResolved(_)
        | EventKind::TranscriptPartUpserted(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::UserInputRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::ToolPolicyDenied(_)
        | EventKind::ToolUserDeclined(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::BackgroundActivityChanged(_)
        | EventKind::PluginEvent(_)
        | EventKind::PluginToolRegistryChanged(_)
        | EventKind::ActivityV2(_)
        | EventKind::ToolCallIssued(_) => {}
        EventKind::ToolCallCompleted(_) => {}
    }
}

/// Rewrite every `message_id` in `kind` through `f`. Mirror of
/// [`visit_event_message_ids`].
pub(crate) fn rewrite_event_message_ids(kind: &mut EventKind, mut f: impl FnMut(i64) -> i64) {
    use agena_domain::MessageId;
    match kind {
        EventKind::UserMessageAppended(p) => {
            p.message_id = MessageId(f(p.message_id.raw()));
            rewrite_message_metadata_ids(&mut p.metadata, &mut f);
            for part in &mut p.parts {
                part.message_id = f(part.message_id);
            }
        }
        EventKind::AssistantMessageFinished(p) => {
            p.message_id = MessageId(f(p.message_id.raw()));
            rewrite_message_metadata_ids(&mut p.metadata, &mut f);
            for part in &mut p.parts {
                part.message_id = f(part.message_id);
            }
        }
        EventKind::ToolCallIssued(p) => {
            p.message_id = MessageId(f(p.message_id.raw()));
        }
        EventKind::ToolCallCompleted(p) => {
            p.message_id = MessageId(f(p.message_id.raw()));
        }
        EventKind::CompactionCompleted(p) => {
            p.activity.compacted_through_message_id = f(p.activity.compacted_through_message_id);
        }
        EventKind::MessagePartCheckpointed(p) => {
            p.message_id = f(p.message_id);
            rewrite_message_metadata_ids(&mut p.message_metadata, &mut f);
            p.part.message_id = f(p.part.message_id);
        }
        EventKind::ExecutionStarted(_)
        | EventKind::ExecutionFinished(_)
        | EventKind::SubtaskStatusChanged(_)
        | EventKind::StreamError(_)
        | EventKind::ProviderRetry(_)
        | EventKind::ProviderRetryResolved(_)
        | EventKind::TranscriptPartUpserted(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::UserInputRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::ToolPolicyDenied(_)
        | EventKind::ToolUserDeclined(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::BackgroundActivityChanged(_)
        | EventKind::PluginEvent(_)
        | EventKind::PluginToolRegistryChanged(_)
        | EventKind::ActivityV2(_) => {}
    }
}

pub(crate) fn rewrite_message_metadata_ids(
    metadata: &mut crate::message::MessageMetadata,
    mut f: impl FnMut(i64) -> i64,
) {
    if let Some(turn_id) = metadata.model_turn_id.as_mut() {
        *turn_id = f(*turn_id);
    }
    if let Some(parent_message_id) = metadata.parent_message_id.as_mut() {
        *parent_message_id = f(*parent_message_id);
    }
}

/// Rewrite every `part_id` in `kind` through `f`. Mirror of
/// [`visit_event_part_ids`].
pub(crate) fn rewrite_event_part_ids(kind: &mut EventKind, mut f: impl FnMut(i64) -> i64) {
    match kind {
        EventKind::UserMessageAppended(p) => {
            for part in &mut p.parts {
                part.id = f(part.id);
            }
        }
        EventKind::AssistantMessageFinished(p) => {
            for part in &mut p.parts {
                part.id = f(part.id);
            }
        }
        EventKind::MessagePartCheckpointed(p) => {
            p.part.id = f(p.part.id);
        }
        EventKind::CompactionCompleted(_) => {}
        EventKind::ExecutionStarted(_)
        | EventKind::ExecutionFinished(_)
        | EventKind::SubtaskStatusChanged(_)
        | EventKind::StreamError(_)
        | EventKind::ProviderRetry(_)
        | EventKind::ProviderRetryResolved(_)
        | EventKind::TranscriptPartUpserted(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::UserInputRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::ToolPolicyDenied(_)
        | EventKind::ToolUserDeclined(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::BackgroundActivityChanged(_)
        | EventKind::ToolCallIssued(_)
        | EventKind::PluginEvent(_)
        | EventKind::PluginToolRegistryChanged(_)
        | EventKind::ActivityV2(_) => {}
        EventKind::ToolCallCompleted(_) => {}
    }
}

pub(crate) fn rewrite_event_session_ids(kind: &mut EventKind, session_id: i64) {
    match kind {
        EventKind::ExecutionStarted(p) => p.session_id = session_id,
        EventKind::ExecutionFinished(p) => p.session_id = session_id,
        EventKind::CompactionCompleted(p) => p.session_id = session_id,
        EventKind::SubtaskStatusChanged(p) => p.session_id = session_id,
        EventKind::StreamError(p) => p.session_id = session_id,
        EventKind::ProviderRetry(p) => p.session_id = session_id,
        EventKind::ProviderRetryResolved(p) => p.session_id = session_id,
        EventKind::MessagePartCheckpointed(p) => p.session_id = session_id,
        EventKind::TranscriptPartUpserted(p) => p.session_id = session_id,
        EventKind::CommandBegin(p) => p.context.session_id = session_id,
        EventKind::CommandOutputDelta(p) => p.context.session_id = session_id,
        EventKind::CommandEnd(p) => p.context.session_id = session_id,
        EventKind::PermissionRequested(p) => p.session_id = session_id,
        EventKind::UserInputRequested(p) => p.session_id = session_id,
        EventKind::PermissionReplied(p) => p.session_id = session_id,
        EventKind::PermissionRuleCreated(p)
        | EventKind::PermissionRuleUpdated(p)
        | EventKind::PermissionRuleRevoked(p) => {
            if p.session_id.is_some() {
                p.session_id = Some(session_id);
            }
        }
        EventKind::ToolPolicyDenied(p) => p.session_id = session_id,
        EventKind::ToolUserDeclined(p) => p.session_id = session_id,
        EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::UserMessageAppended(_)
        | EventKind::AssistantMessageFinished(_)
        | EventKind::ToolCallIssued(_)
        | EventKind::ToolCallCompleted(_)
        | EventKind::BackgroundActivityChanged(_)
        | EventKind::PluginEvent(_)
        | EventKind::PluginToolRegistryChanged(_)
        | EventKind::ActivityV2(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{rewrite_message_metadata_ids, visit_message_metadata_ids};

    #[test]
    fn turn_identity_participates_in_import_id_visiting_and_rewriting() {
        let mut metadata = crate::message::MessageMetadata {
            model_turn_id: Some(7),
            parent_message_id: Some(6),
            ..Default::default()
        };
        let mut visited = Vec::new();
        visit_message_metadata_ids(&metadata, |id| visited.push(id));
        assert_eq!(visited, vec![7, 6]);

        rewrite_message_metadata_ids(&mut metadata, |id| id + 100);
        assert_eq!(metadata.model_turn_id, Some(107));
        assert_eq!(metadata.parent_message_id, Some(106));
    }
}
