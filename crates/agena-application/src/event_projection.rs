use agena_api::{EventKindTag, EventMetaResource, EventResource};

/// Projects a runtime event into the public API envelope without allowing the
/// wire contract to depend on the runtime event enum.
pub fn event_resource_from_runtime(event: &agena_runtime::RuntimeEvent) -> EventResource {
    EventResource {
        meta: EventMetaResource {
            id: event.meta.id,
            seq_global: event.meta.seq_global,
            seq_session: event.meta.seq_session,
            session_id: event.meta.session_id,
            workspace_id: event.meta.workspace_id,
            created_at: event.meta.created_at,
            causation_id: event.meta.causation_id,
            correlation_id: event.meta.correlation_id,
            envelope_schema: event.meta.envelope_schema,
        },
        kind: EventKindTag::new(event.kind.clone()),
        payload: event.payload.clone(),
    }
}

#[cfg(test)]
mod tests {
    use agena_domain::EventMeta;
    use chrono::Utc;
    use uuid::Uuid;

    use super::event_resource_from_runtime;

    #[test]
    fn preserves_the_runtime_event_wire_shape_without_exposing_its_type() {
        let event = agena_runtime::RuntimeEvent {
            meta: EventMeta {
                id: Uuid::nil(),
                seq_global: 3,
                seq_session: Some(2),
                session_id: Some(7),
                workspace_id: None,
                created_at: Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: 1,
            },
            kind: "execution_started".to_owned(),
            payload: serde_json::json!({"session_id": 7, "ts_ms": 42}),
            invalidates_ancestor_projection: false,
        };
        let resource = event_resource_from_runtime(&event);
        assert_eq!(resource.kind.as_str(), "execution_started");
        assert_eq!(resource.payload["session_id"], 7);
        assert_eq!(resource.meta.seq_global, 3);
    }
}
