use std::collections::{HashMap, HashSet};

use agena_storage::kernel::{
    ExternalAgentCursorChange, ExternalAgentCursorEntity, ExternalAgentNativeCursor,
};
use serde_json::Value;

use super::{MAX_CURSOR_BYTES, bounded_string};

pub(super) const REWIND_CAPABILITY: &str = "sessionRewind";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Entity {
    kind: &'static str,
    id: String,
}

impl Entity {
    fn update_kind(update: &Value) -> Option<&'static str> {
        Some(match update["sessionUpdate"].as_str()? {
            "user_message" | "user_message_chunk" => "user_message",
            "agent_message" | "agent_message_chunk" => "agent_message",
            "agent_thought" | "agent_thought_chunk" => "agent_thought",
            "session_message" | "session_message_chunk" => "session_message",
            "tool_call" | "tool_call_update" | "tool_call_content_chunk" => "tool_call",
            "compaction_update" | "compaction_summary_chunk" => "compaction",
            _ => return None,
        })
    }

    pub(super) fn from_update(update: &Value) -> Option<Self> {
        Self::new(Self::update_kind(update)?, update)
    }

    pub(super) fn message(kind: &'static str, id: &str) -> Self {
        Self {
            kind,
            id: id.to_owned(),
        }
    }

    fn new(kind: &'static str, value: &Value) -> Option<Self> {
        let field = match kind {
            "tool_call" => "toolCallId",
            "compaction" => "compactionId",
            _ => "messageId",
        };
        Some(Self {
            kind,
            id: bounded_string(&value[field], MAX_CURSOR_BYTES)?,
        })
    }

    fn kind(value: &str) -> Option<&'static str> {
        Some(match value {
            "user_message" => "user_message",
            "agent_message" => "agent_message",
            "agent_thought" => "agent_thought",
            "session_message" => "session_message",
            "tool_call" => "tool_call",
            "compaction" => "compaction",
            _ => return None,
        })
    }

    pub(super) fn restore(entity: ExternalAgentCursorEntity) -> Option<Self> {
        Some(Self {
            kind: Self::kind(&entity.kind)?,
            id: entity.id,
        })
    }

    pub(super) fn saved(self) -> ExternalAgentCursorEntity {
        ExternalAgentCursorEntity {
            kind: self.kind.to_owned(),
            id: self.id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Anchor {
    after: bool,
    entity: Entity,
}

impl Anchor {
    fn parse(value: &Value) -> Option<Self> {
        let kind = Entity::kind(value["type"].as_str()?)?;
        let after = match value["position"].as_str()? {
            "before" => false,
            "after" => true,
            _ => return None,
        };
        Some(Self {
            after,
            entity: Entity::new(kind, value)?,
        })
    }

    fn before_user(&self) -> Option<String> {
        (!self.after && self.entity.kind == "user_message").then(|| self.entity.id.clone())
    }
}

#[derive(Default)]
pub(super) struct Tail {
    latest: Option<Entity>,
    seen: HashSet<Entity>,
}

impl Tail {
    pub(super) fn new(previous: Option<Entity>) -> Self {
        Self {
            seen: previous.iter().cloned().collect(),
            latest: previous,
        }
    }

    pub(super) fn observe(&mut self, update: &Value) {
        if Entity::update_kind(update).is_none() {
            return;
        }
        match Entity::from_update(update) {
            Some(entity) if self.seen.contains(&entity) => {}
            Some(entity) if self.seen.len() < 2048 => {
                self.seen.insert(entity.clone());
                self.latest = Some(entity);
            }
            _ => self.latest = None,
        }
    }

    pub(super) fn latest(&self) -> Option<ExternalAgentCursorEntity> {
        self.latest.clone().map(Entity::saved)
    }
}

fn cursor_id(update: &Value) -> Option<String> {
    if update["sessionUpdate"] != "cursor_update" {
        return None;
    }
    bounded_string(&update["cursorId"], MAX_CURSOR_BYTES)
}

pub(super) fn anchored_entity(update: &Value) -> Option<Entity> {
    Some(Anchor::parse(&update["anchor"])?.entity)
}

#[derive(Clone, Copy, Default)]
struct Grant {
    fork: bool,
    rewind: bool,
}

impl Grant {
    fn from_update(update: &Value, allow_rewind: bool) -> Option<Self> {
        let value = update.get("capabilities")?;
        if !value.is_null() && !value.is_object() {
            return None;
        }
        let supports = |key| value[key].is_object() || value[key] == true;
        Some(Self {
            fork: supports("fork"),
            rewind: allow_rewind && supports("rewind"),
        })
    }
}

pub(super) fn change(update: &Value, allow_rewind: bool) -> Option<ExternalAgentCursorChange> {
    let grant = Grant::from_update(update, allow_rewind)?;
    Some(ExternalAgentCursorChange {
        cursor_id: cursor_id(update)?,
        before_user: Anchor::parse(&update["anchor"]).and_then(|anchor| anchor.before_user()),
        fork: grant.fork,
        rewind: grant.rewind,
    })
}

/// Cursor anchors are immutable. Repeated updates may only replace capability
/// flags, including clearing them without sending the anchor again.
#[derive(Default)]
pub(super) struct Book {
    grants: HashMap<String, Grant>,
    anchors: HashMap<Anchor, Vec<String>>,
}

impl Book {
    pub(super) fn forget(&mut self, entity: &Entity) {
        for after in [false, true] {
            if let Some(ids) = self.anchors.remove(&Anchor {
                after,
                entity: entity.clone(),
            }) {
                for id in ids {
                    self.grants.remove(&id);
                }
            }
        }
    }

    pub(super) fn observe(&mut self, update: &Value, allow_rewind: bool, limit: usize) {
        let Some(id) = cursor_id(update) else { return };
        let grant = Grant::from_update(update, allow_rewind);
        if let Some(current) = self.grants.get_mut(&id) {
            if let Some(grant) = grant {
                *current = grant;
            }
        } else if self.grants.len() < limit
            && let Some(anchor) = Anchor::parse(&update["anchor"])
        {
            self.anchors.entry(anchor).or_default().push(id.clone());
            self.grants.insert(id, grant.unwrap_or_default());
        }
    }

    pub(super) fn for_user(
        &self,
        message: Option<&str>,
        previous: Option<&Entity>,
    ) -> Option<ExternalAgentNativeCursor> {
        let before = message.map(|id| Anchor {
            after: false,
            entity: Entity::message("user_message", id),
        });
        let after = previous.map(|entity| Anchor {
            after: true,
            entity: entity.clone(),
        });
        let mut candidates = before
            .iter()
            .chain(after.iter())
            .filter_map(|anchor| self.anchors.get(anchor))
            .flat_map(|ids| ids.iter().rev())
            .filter_map(|id| self.grants.get(id).map(|grant| (id, grant)));
        let selected = candidates
            .clone()
            .find(|(_, grant)| grant.fork || grant.rewind)
            .or_else(|| candidates.next());
        if selected.is_none() && message.is_none() {
            return None;
        }
        let (cursor_id, grant) = selected
            .map(|(id, grant)| (Some(id.clone()), *grant))
            .unwrap_or_default();
        Some(ExternalAgentNativeCursor {
            message_id: message.unwrap_or_default().to_owned(),
            cursor_id,
            fork: grant.fork,
            rewind: grant.rewind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tail_observe_preserves_latest_after_seen_cap_is_reached() {
        let mut tail = Tail::default();
        for i in 0..2048 {
            tail.observe(&json!({
                "sessionUpdate": "agent_message",
                "messageId": format!("msg-{i}")
            }));
        }
        assert_eq!(
            tail.latest().map(|entity| entity.id),
            Some("msg-2047".to_owned())
        );
        // Re-observing an already-seen entity at capacity must not wipe latest.
        tail.observe(&json!({
            "sessionUpdate": "agent_message_chunk",
            "messageId": "msg-2047"
        }));
        assert_eq!(
            tail.latest().map(|entity| entity.id),
            Some("msg-2047".to_owned())
        );
        // An unseen 2049th entity exceeds the cap and safely clears latest.
        tail.observe(&json!({
            "sessionUpdate": "agent_message",
            "messageId": "msg-2048"
        }));
        assert!(tail.latest().is_none());
    }
}
