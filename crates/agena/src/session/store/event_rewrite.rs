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
            visit_persistent_message_ids(p.message_id.raw(), &p.metadata, &p.parts, &mut visit);
        }
        EventKind::AssistantMessageFinished(p) => {
            visit_persistent_message_ids(p.message_id.raw(), &p.metadata, &p.parts, &mut visit);
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
    if let Some(parent_message_id) = metadata.parent_message_id {
        visit(parent_message_id);
    }
}

/// Visit the message id, parent id, and part ownership ids carried by either
/// persistent message event. User and assistant history events differ in
/// their content fields, but share this durable id topology.
fn visit_persistent_message_ids(
    message_id: i64,
    metadata: &crate::message::MessageMetadata,
    parts: &[crate::message::MessagePart],
    visit: &mut impl FnMut(i64),
) {
    visit(message_id);
    visit_message_metadata_ids(metadata, &mut *visit);
    for part in parts {
        visit(part.message_id);
    }
}

fn visit_message_part_ids(parts: &[crate::message::MessagePart], visit: &mut impl FnMut(i64)) {
    for part in parts {
        visit(part.id);
    }
}

pub(crate) fn visit_event_part_ids(kind: &EventKind, mut visit: impl FnMut(i64)) {
    match kind {
        EventKind::UserMessageAppended(p) => {
            visit_message_part_ids(&p.parts, &mut visit);
        }
        EventKind::AssistantMessageFinished(p) => {
            visit_message_part_ids(&p.parts, &mut visit);
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
        | EventKind::ToolCallIssued(_)
        | EventKind::SystemNoticeAppended(_) => {}
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
            rewrite_persistent_message_ids(
                &mut p.message_id,
                &mut p.metadata,
                &mut p.parts,
                &mut f,
            );
        }
        EventKind::AssistantMessageFinished(p) => {
            rewrite_persistent_message_ids(
                &mut p.message_id,
                &mut p.metadata,
                &mut p.parts,
                &mut f,
            );
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
    if let Some(parent_message_id) = metadata.parent_message_id.as_mut() {
        *parent_message_id = f(*parent_message_id);
    }
}

/// Rewrite the durable id topology shared by both persistent message events.
fn rewrite_persistent_message_ids(
    message_id: &mut crate::session::ids::MessageId,
    metadata: &mut crate::message::MessageMetadata,
    parts: &mut [crate::message::MessagePart],
    rewrite: &mut impl FnMut(i64) -> i64,
) {
    *message_id = crate::session::ids::MessageId(rewrite(message_id.raw()));
    rewrite_message_metadata_ids(metadata, &mut *rewrite);
    for part in parts {
        part.message_id = rewrite(part.message_id);
    }
}

fn rewrite_message_part_ids(
    parts: &mut [crate::message::MessagePart],
    rewrite: &mut impl FnMut(i64) -> i64,
) {
    for part in parts {
        part.id = rewrite(part.id);
    }
}

/// Rewrite every `part_id` in `kind` through `f`. Mirror of
/// [`visit_event_part_ids`].
pub(crate) fn rewrite_event_part_ids(kind: &mut EventKind, mut f: impl FnMut(i64) -> i64) {
    match kind {
        EventKind::UserMessageAppended(p) => {
            rewrite_message_part_ids(&mut p.parts, &mut f);
        }
        EventKind::AssistantMessageFinished(p) => {
            rewrite_message_part_ids(&mut p.parts, &mut f);
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
        | EventKind::SystemNoticeAppended(_)
        | EventKind::PluginEvent(_)
        | EventKind::PluginToolRegistryChanged(_) => {}
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
