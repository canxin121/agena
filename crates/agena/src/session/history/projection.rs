use crate::{message::Message, session::SessionRuntimeState};

use super::{
    HistoryItem, LegacySnapshotImported, LoadedDeferredToolsRecorded, MessageSnapshotRecorded,
    ProviderAnchorSet, ProviderAnchorsCleared, PromptTokensRecorded, SessionRuntimeRecorded,
};

pub(crate) fn history_items_from_message_snapshot(message: &Message) -> Vec<HistoryItem> {
    vec![HistoryItem::MessageSnapshotRecorded(MessageSnapshotRecorded {
        message: message.clone(),
    })]
}

pub(crate) fn history_items_from_legacy_snapshot(messages: &[Message]) -> Vec<HistoryItem> {
    let mut items = messages
        .iter()
        .flat_map(history_items_from_message_snapshot)
        .collect::<Vec<_>>();
    items.push(HistoryItem::LegacySnapshotImported(LegacySnapshotImported {
        message_count: messages.len(),
    }));
    items
}

pub(crate) fn history_items_from_runtime_diff(
    previous: &SessionRuntimeState,
    next: &SessionRuntimeState,
) -> Vec<HistoryItem> {
    if previous != next {
        return vec![HistoryItem::SessionRuntimeRecorded(SessionRuntimeRecorded {
            runtime: next.clone(),
        })];
    }

    let mut items = Vec::new();

    if previous.prompt_tokens != next.prompt_tokens {
        items.push(HistoryItem::PromptTokensRecorded(PromptTokensRecorded {
            runtime: next.prompt_tokens.clone(),
        }));
    }

    if previous.provider_anchors != next.provider_anchors {
        if next.provider_anchors.is_empty() {
            items.push(HistoryItem::ProviderAnchorsCleared(ProviderAnchorsCleared));
        } else {
            for anchor in next.provider_anchors.values() {
                items.push(HistoryItem::ProviderAnchorSet(ProviderAnchorSet {
                    anchor: anchor.clone(),
                }));
            }
        }
    }

    if previous.loaded_deferred_tools != next.loaded_deferred_tools {
        items.push(HistoryItem::LoadedDeferredToolsRecorded(
            LoadedDeferredToolsRecorded {
                tools: next.loaded_deferred_tools.clone(),
            },
        ));
    }

    items
}
