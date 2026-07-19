use super::{EventKind, Message, Session};

pub(crate) fn event_targets_message(kind: &EventKind, message_id: i64) -> bool {
    match kind {
        EventKind::UserMessageAppended(payload) => payload.message_id.raw() == message_id,
        EventKind::AssistantMessageFinished(payload) => payload.message_id.raw() == message_id,
        EventKind::ToolCallIssued(payload) => payload.message_id.raw() == message_id,
        EventKind::ToolCallCompleted(payload) => payload.message_id.raw() == message_id,
        EventKind::SystemNoticeAppended(payload) => payload.message_id.raw() == message_id,
        _ => false,
    }
}

pub(crate) fn event_run_id_for_message(
    kind: &EventKind,
    message_id: i64,
) -> Option<crate::session::ids::RunId> {
    match kind {
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
            visit(p.part.message_id);
        }
        EventKind::SystemNoticeAppended(p) => {
            visit(p.message_id.raw());
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
        | EventKind::MessagePartDelta(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::PluginEvent(_)
        | EventKind::PluginToolRegistryChanged(_) => {}
    }
}

pub(crate) fn visit_message_metadata_ids(
    metadata: &crate::message::MessageMetadata,
    mut visit: impl FnMut(i64),
) {
    if let Some(turn_id) = metadata.turn_id {
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
        EventKind::ExecutionStarted(_)
        | EventKind::ExecutionFinished(_)
        | EventKind::SubtaskStatusChanged(_)
        | EventKind::StreamError(_)
        | EventKind::MessagePartDelta(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::PluginEvent(_)
        | EventKind::PluginToolRegistryChanged(_)
        | EventKind::ToolCallIssued(_) => {}
        EventKind::SystemNoticeAppended(p) => visit(p.part_id.raw()),
        EventKind::ToolCallCompleted(p) => {
            visit(p.part.id);
        }
    }
}

/// Rewrite every `message_id` in `kind` through `f`. Mirror of
/// [`visit_event_message_ids`].
pub(crate) fn rewrite_event_message_ids(kind: &mut EventKind, mut f: impl FnMut(i64) -> i64) {
    use crate::session::ids::MessageId;
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
            p.part.message_id = f(p.part.message_id);
        }
        EventKind::SystemNoticeAppended(p) => {
            p.message_id = MessageId(f(p.message_id.raw()));
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
        | EventKind::MessagePartDelta(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::PluginEvent(_)
        | EventKind::PluginToolRegistryChanged(_) => {}
    }
}

pub(crate) fn rewrite_message_metadata_ids(
    metadata: &mut crate::message::MessageMetadata,
    mut f: impl FnMut(i64) -> i64,
) {
    if let Some(turn_id) = metadata.turn_id.as_mut() {
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
        EventKind::ExecutionStarted(_)
        | EventKind::ExecutionFinished(_)
        | EventKind::SubtaskStatusChanged(_)
        | EventKind::StreamError(_)
        | EventKind::MessagePartDelta(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::ToolCallIssued(_)
        | EventKind::PluginEvent(_)
        | EventKind::PluginToolRegistryChanged(_) => {}
        EventKind::SystemNoticeAppended(p) => {
            p.part_id = crate::session::ids::PartId(f(p.part_id.raw()));
        }
        EventKind::ToolCallCompleted(p) => {
            p.part.id = f(p.part.id);
        }
    }
}

pub(crate) fn rewrite_event_session_ids(kind: &mut EventKind, session_id: i64) {
    match kind {
        EventKind::ExecutionStarted(p) => p.session_id = session_id,
        EventKind::ExecutionFinished(p) => p.session_id = session_id,
        EventKind::SubtaskStatusChanged(p) => p.session_id = session_id,
        EventKind::StreamError(p) => p.session_id = session_id,
        EventKind::MessagePartCheckpointed(p) => p.session_id = session_id,
        EventKind::MessagePartDelta(p) => p.session_id = session_id,
        EventKind::CommandBegin(p) => p.context.session_id = session_id,
        EventKind::CommandOutputDelta(p) => p.context.session_id = session_id,
        EventKind::CommandEnd(p) => p.context.session_id = session_id,
        EventKind::PermissionRequested(p) => p.session_id = session_id,
        EventKind::PermissionReplied(p) => p.session_id = session_id,
        EventKind::PermissionRuleCreated(p)
        | EventKind::PermissionRuleUpdated(p)
        | EventKind::PermissionRuleRevoked(p) => {
            if p.session_id.is_some() {
                p.session_id = Some(session_id);
            }
        }
        EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::UserMessageAppended(_)
        | EventKind::AssistantMessageFinished(_)
        | EventKind::ToolCallIssued(_)
        | EventKind::ToolCallCompleted(_)
        | EventKind::SystemNoticeAppended(_)
        | EventKind::PluginEvent(_)
        | EventKind::PluginToolRegistryChanged(_) => {}
    }
}

pub(crate) fn ordered_unique_touched_messages(
    session: &Session,
    touched_messages: Vec<Message>,
) -> Vec<Message> {
    let session_order = session
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| (message.id, index))
        .collect::<std::collections::HashMap<_, _>>();

    let mut latest_by_id = std::collections::HashMap::new();
    for message in touched_messages {
        latest_by_id.insert(message.id, message);
    }

    let mut ordered = latest_by_id.into_values().collect::<Vec<_>>();
    ordered.sort_by_key(|message| {
        (
            session_order
                .get(&message.id)
                .copied()
                .unwrap_or(usize::MAX),
            message.id,
        )
    });
    ordered
}

#[cfg(test)]
mod tests {
    use super::{rewrite_message_metadata_ids, visit_message_metadata_ids};

    #[test]
    fn turn_identity_participates_in_import_id_visiting_and_rewriting() {
        let mut metadata = crate::message::MessageMetadata {
            turn_id: Some(7),
            parent_message_id: Some(6),
            ..Default::default()
        };
        let mut visited = Vec::new();
        visit_message_metadata_ids(&metadata, |id| visited.push(id));
        assert_eq!(visited, vec![7, 6]);

        rewrite_message_metadata_ids(&mut metadata, |id| id + 100);
        assert_eq!(metadata.turn_id, Some(107));
        assert_eq!(metadata.parent_message_id, Some(106));
    }
}
