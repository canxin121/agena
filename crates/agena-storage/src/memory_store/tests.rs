use super::*;
use crate::{MemoryKind, MemoryOrigin, MemoryRevision, MemorySource};

fn note(name: &str, body: &str) -> NewMemory {
    NewMemory {
        name: name.into(),
        description: "A project decision".into(),
        kind: MemoryKind::Fact,
        body: body.into(),
    }
}

fn source(entry_id: i64) -> MemoryWriteOptions {
    MemoryWriteOptions {
        source: Some(MemorySource {
            instance_id: "runtime-a".into(),
            session_id: 42,
            call_id: Some(format!("call-{entry_id}")),
            entry_id: Some(entry_id),
            ..MemorySource::default()
        }),
        ..MemoryWriteOptions::default()
    }
}

#[test]
fn nonpositive_source_and_scope_identities_cannot_publish() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    for invalid_id in [-1, 0] {
        let mut scope = source(1);
        scope.scope = Some(MemoryScope::Session {
            session_id: invalid_id,
            instance_id: "runtime-a".into(),
        });
        let mut session = source(1);
        session.source.as_mut().unwrap().session_id = invalid_id;
        for (field, options) in [
            ("scope", scope),
            ("source-session", session),
            ("source-entry", source(invalid_id)),
        ] {
            let name = format!("{field}-{invalid_id}");
            let snapshot = store.write_snapshot(&name).unwrap();
            assert!(
                store
                    .save_with_options(note(&name, "Invalid identity"), None, options.clone())
                    .is_err(),
                "explicit {field}={invalid_id} must be rejected"
            );
            assert!(
                store
                    .save_derived(note(&name, "Invalid identity"), &snapshot, options)
                    .is_err(),
                "derived {field}={invalid_id} must be rejected"
            );
            assert!(!store.resolve_path(&name).exists());
        }
    }
    assert!(store.list().unwrap().is_empty());
    assert!(!store.metadata_path().exists());
    assert!(!dir.path().join(ENTRYPOINT_NAME).exists());
    assert!(!dir.path().join(transaction::LOCK_NAME).exists());
}

#[test]
fn rejects_legacy_or_incomplete_documents_without_creating_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let documents = [
        ("plain", "A note without a header.\n"),
        ("empty", "---\n\n---\n\nMissing header fields.\n"),
        (
            "legacy",
            "---\nname: legacy\ndescription: 用户偏好\ntype: user\n---\n\n用中文回答。\n",
        ),
        (
            "missing-revision",
            "---\nschema_version: 2\nname: missing-revision\ndescription: incomplete\n---\n\nNo revision.\n",
        ),
        (
            "unregistered",
            "---\nschema_version: 2\nname: unregistered\ndescription: unmanaged\nrevision: 1\n---\n\nNo lifecycle state.\n",
        ),
    ];
    for (name, raw) in documents {
        let path = dir.path().join(format!("{name}.md"));
        fs::write(&path, raw).unwrap();
        assert!(store.get(name).is_err(), "{name} must not become a memory");
        assert!(store.write_snapshot(name).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), raw);
    }
    assert!(store.list().unwrap().is_empty());
    assert!(!store.metadata_path().exists());
    assert!(!dir.path().join(transaction::LOCK_NAME).exists());
}

#[test]
fn noncanonical_disk_names_cannot_alias_registered_records() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let original = store
        .save_with_options(note("foo", "Canonical source memory"), None, source(1))
        .unwrap();
    let original_raw = fs::read_to_string(&original.path).unwrap();
    let state_bytes = fs::read(store.metadata_path()).unwrap();
    let alias_path = dir.path().join("foo.md.md");
    let alias_raw = format!(
        "---\nschema_version: 2\nname: foo.md\ndescription: Alias\nrevision: {}\n---\n\nAlias content must not enter recall.\n",
        original.frontmatter.revision
    );
    fs::write(&alias_path, &alias_raw).unwrap();
    assert!(read_entry(&alias_path).is_err());

    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].frontmatter.name, "foo");
    assert_eq!(listed[0].file_name, "foo.md");
    assert_eq!(listed[0].body, original.body);
    assert_eq!(listed[0].sha256, original.sha256);
    assert_eq!(listed[0].metadata, original.metadata);
    assert_eq!(store.get("foo").unwrap().sha256, original.sha256);
    assert_eq!(fs::read_to_string(&original.path).unwrap(), original_raw);
    assert_eq!(fs::read(store.metadata_path()).unwrap(), state_bytes);
    assert_eq!(fs::read_to_string(alias_path).unwrap(), alias_raw);
}

#[test]
fn create_only_writes_and_kind_changes_require_the_current_revision() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let first = store
        .save_with_options(
            note("decision", "Same body"),
            None,
            MemoryWriteOptions::default(),
        )
        .unwrap();
    assert_eq!(first.frontmatter.schema_version, 2);
    assert_eq!(first.frontmatter.revision, first.metadata.revision);
    let changed = NewMemory {
        kind: MemoryKind::Preference,
        ..note("decision", "Same body")
    };
    assert!(
        store
            .save_with_options(changed.clone(), None, MemoryWriteOptions::default())
            .is_err()
    );
    assert!(store.forget_checked("decision", "").is_err());
    assert_eq!(store.get("decision").unwrap().sha256, first.sha256);
    let second = store
        .save_with_options(changed, Some(&first.sha256), MemoryWriteOptions::default())
        .unwrap();
    assert_eq!(second.metadata.kind, MemoryKind::Preference);
    assert_eq!(
        MemoryStore::at(dir.path())
            .get("decision")
            .unwrap()
            .metadata
            .kind,
        MemoryKind::Preference
    );
    assert_ne!(first.sha256, second.sha256);
}

#[test]
fn old_state_and_document_schema_versions_are_rejected_without_migration() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let first = store
        .save_with_options(note("decision", "Current fact"), None, source(1))
        .unwrap();
    let state_bytes = fs::read(store.metadata_path()).unwrap();
    let mut old_state: serde_json::Value = serde_json::from_slice(&state_bytes).unwrap();
    assert_eq!(old_state["schema_version"], 2);
    old_state["schema_version"] = serde_json::json!(1);
    let unsupported = serde_json::to_vec(&old_state).unwrap();
    fs::write(store.metadata_path(), &unsupported).unwrap();
    assert!(store.get("decision").is_err());
    assert!(
        store
            .save_with_options(note("decision", "Rejected"), Some(&first.sha256), source(2))
            .is_err()
    );
    assert_eq!(fs::read(store.metadata_path()).unwrap(), unsupported);
    fs::write(store.metadata_path(), state_bytes).unwrap();
    let old_document = fs::read_to_string(&first.path)
        .unwrap()
        .replace("schema_version: 2", "schema_version: 1");
    fs::write(&first.path, &old_document).unwrap();
    assert!(store.get("decision").is_err());
    assert!(store.write_snapshot("decision").is_err());
    assert_eq!(fs::read_to_string(&first.path).unwrap(), old_document);
}

#[test]
fn lifecycle_metadata_cannot_be_deserialized_by_filling_required_fields() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let record = store
        .save_with_options(note("decision", "Fact"), None, source(1))
        .unwrap();
    let full = serde_json::to_value(record.metadata).unwrap();
    for key in [
        "id",
        "revision",
        "kind",
        "scope",
        "status",
        "origin",
        "importance",
        "created_at_ms",
        "updated_at_ms",
        "provenance_verified",
        "source_suppressed",
    ] {
        let mut incomplete = full.clone();
        incomplete.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<MemoryMetadata>(incomplete).is_err(),
            "missing {key} must be rejected"
        );
    }
    assert!(serde_json::from_value::<MemoryMetadata>(full).is_ok());
}

#[test]
fn stamps_sources_and_metadata_changes_invalidate_old_sha() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let first = store
        .save_with_options(note("decision", "Use PostgreSQL."), None, source(1))
        .unwrap();
    assert_eq!(first.metadata.origin, MemoryOrigin::Explicit);
    assert!(first.metadata.provenance_verified);
    assert!(first.metadata.created_at_ms > 0);
    assert_eq!(first.frontmatter.revision, first.metadata.revision);
    assert_eq!(first.metadata.source.as_ref().unwrap().entry_id, Some(1));
    let mut options = source(2);
    options.importance = Some(90);
    let second = store
        .save_with_options(
            note("decision", "Use PostgreSQL."),
            Some(&first.sha256),
            options,
        )
        .unwrap();
    assert_eq!(first.metadata.id, second.metadata.id);
    assert_eq!(first.metadata.created_at_ms, second.metadata.created_at_ms);
    assert_ne!(first.sha256, second.sha256);
    assert!(second.metadata.revision > first.metadata.revision);
    assert_eq!(second.metadata.importance, 90);
    assert!(
        store
            .save_with_options(
                note("decision", "stale"),
                Some(&first.sha256),
                MemoryWriteOptions::default()
            )
            .is_err()
    );
    let reopened = MemoryStore::at(dir.path()).get("decision").unwrap();
    assert_eq!(reopened.metadata, second.metadata);
    assert_eq!(reopened.sha256, second.sha256);
}

#[test]
fn correction_keeps_history_but_removes_old_note_from_current_index() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let old = store
        .save_with_options(note("old-database", "Use SQLite."), None, source(1))
        .unwrap();
    let mut options = source(2);
    options.supersedes = vec![MemoryRevision {
        name: "old-database".into(),
        sha256: old.sha256.clone(),
    }];
    let new = store
        .save_with_options(note("database", "Use PostgreSQL."), None, options)
        .unwrap();
    let history = store.get("old-database").unwrap();
    assert!(history.body.contains("SQLite"));
    assert_eq!(history.metadata.status, MemoryStatus::Superseded);
    assert_eq!(
        history.metadata.superseded_by.as_deref(),
        Some(new.metadata.id.as_str())
    );
    assert!(new.metadata.supersedes.contains(&history.metadata.id));
    assert!(!history.is_active_in_context(Some(42), Some("runtime-a")));
    assert!(new.is_active_in_context(Some(42), Some("runtime-a")));
    assert_ne!(old.sha256, history.sha256);
    let index = store.index_lines().unwrap().join("\n");
    assert!(index.contains("(database.md)"));
    assert!(!index.contains("(old-database.md)"));
    assert_eq!(store.list().unwrap().len(), 2);
    assert!(store.forget_checked("old-database", &old.sha256).is_err());
}

#[test]
fn failed_multi_record_correction_publishes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let a = store
        .save_with_options(note("a", "old A"), None, MemoryWriteOptions::default())
        .unwrap();
    let b = store
        .save_with_options(note("b", "old B"), None, MemoryWriteOptions::default())
        .unwrap();
    let before_index = store.index_lines().unwrap();
    let before_state = fs::read(store.metadata_path()).unwrap();
    let mut options = source(3);
    options.supersedes = vec![
        MemoryRevision {
            name: "a".into(),
            sha256: a.sha256.clone(),
        },
        MemoryRevision {
            name: "b".into(),
            sha256: "stale".into(),
        },
    ];
    assert!(
        store
            .save_with_options(note("combined", "new"), None, options)
            .is_err()
    );
    assert_eq!(store.get("a").unwrap().sha256, a.sha256);
    assert_eq!(store.get("b").unwrap().sha256, b.sha256);
    assert_eq!(store.index_lines().unwrap(), before_index);
    assert_eq!(fs::read(store.metadata_path()).unwrap(), before_state);
    assert!(!dir.path().join("combined.md").exists());
}

#[test]
fn archive_and_session_scope_are_enforced_without_losing_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let mut options = source(1);
    options.scope = Some(MemoryScope::Session {
        session_id: 42,
        instance_id: "runtime-a".into(),
    });
    let task = store
        .save_with_options(note("task", "Finish migration."), None, options)
        .unwrap();
    assert!(task.is_active_in_context(Some(42), Some("runtime-a")));
    assert!(!task.is_active_in_context(Some(43), Some("runtime-a")));
    assert!(!task.is_active_in_context(None, Some("runtime-a")));
    let mut archived = source(2);
    archived.status = Some(MemoryStatus::Archived);
    let task = store
        .save_with_options(
            note("task", "Finish migration."),
            Some(&task.sha256),
            archived,
        )
        .unwrap();
    assert!(!task.is_active_in_context(Some(42), Some("runtime-a")));
    assert!(store.index_lines().unwrap().is_empty());
    assert!(store.get("task").unwrap().body.contains("migration"));
}

#[test]
fn derived_write_rejects_inflight_edit_and_future_explicit_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let snapshot = store.write_snapshot("episode").unwrap();
    let episode = store
        .save_derived(note("episode", "First summary"), &snapshot, source(1))
        .unwrap();
    assert_eq!(episode.metadata.origin, MemoryOrigin::Derived);
    let inflight = store.write_snapshot("episode").unwrap();
    let corrected = store
        .save_with_options(
            note("episode", "User correction"),
            Some(&episode.sha256),
            source(2),
        )
        .unwrap();
    assert!(
        store
            .save_derived(note("episode", "stale result"), &inflight, source(3))
            .is_err()
    );
    let fresh = store.write_snapshot("episode").unwrap();
    assert!(
        store
            .save_derived(note("episode", "overwrite correction"), &fresh, source(4))
            .is_err()
    );
    assert_eq!(store.get("episode").unwrap().sha256, corrected.sha256);
}

#[test]
fn forgotten_memory_stays_forgotten_after_reopen_and_fresh_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let snapshot = store.write_snapshot("episode").unwrap();
    let first = store
        .save_derived(note("episode", "Old summary"), &snapshot, source(1))
        .unwrap();
    let inflight = store.write_snapshot("episode").unwrap();
    store.forget_checked("episode", &first.sha256).unwrap();
    assert!(
        store
            .save_derived(note("episode", "late summary"), &inflight, source(2))
            .is_err()
    );
    let store = MemoryStore::at(dir.path());
    let fresh = store.write_snapshot("episode").unwrap();
    assert!(fresh.is_tombstoned);
    assert!(
        store
            .save_derived(note("episode", "recreated summary"), &fresh, source(3))
            .is_err()
    );
    assert!(store.get("episode").is_err());
    assert!(store.list().unwrap().is_empty());
    assert!(store.index_lines().unwrap().is_empty());
    // Only an intentional explicit create clears the tombstone.
    let recreated = store
        .save_with_options(note("episode", "Explicit restore"), None, source(4))
        .unwrap();
    assert_eq!(recreated.metadata.origin, MemoryOrigin::Explicit);
    assert!(!store.write_snapshot("episode").unwrap().is_tombstoned);
}

#[test]
fn manual_byte_edit_loses_source_attribution_and_protects_from_background() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let snapshot = store.write_snapshot("episode").unwrap();
    let first = store
        .save_derived(note("episode", "Original summary"), &snapshot, source(1))
        .unwrap();
    let raw = fs::read_to_string(&first.path)
        .unwrap()
        .replace("Original summary", "Manual correction");
    fs::write(&first.path, raw).unwrap();
    let edited = store.get("episode").unwrap();
    assert_eq!(edited.metadata.id, first.metadata.id);
    assert_eq!(edited.metadata.origin, MemoryOrigin::External);
    assert!(edited.metadata.source.is_none());
    assert!(!edited.metadata.provenance_verified);
    let fresh = store.write_snapshot("episode").unwrap();
    assert!(
        store
            .save_derived(note("episode", "new generated summary"), &fresh, source(2))
            .is_err()
    );
    assert!(
        store
            .get("episode")
            .unwrap()
            .body
            .contains("Manual correction")
    );
}

#[test]
fn derived_watermark_is_monotonic_and_equal_watermark_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let snapshot = store.write_snapshot("episode").unwrap();
    let first = store
        .save_derived(note("episode", "Summary at entry 5"), &snapshot, source(5))
        .unwrap();
    let fresh = store.write_snapshot("episode").unwrap();
    assert!(
        store
            .save_derived(note("episode", "outdated"), &fresh, source(4))
            .is_err()
    );
    let same = store
        .save_derived(note("episode", "duplicate"), &fresh, source(5))
        .unwrap();
    assert_eq!(same.sha256, first.sha256);
    assert_eq!(
        store.write_snapshot("episode").unwrap().generation,
        fresh.generation
    );
    let next = store
        .save_derived(note("episode", "Summary at entry 6"), &fresh, source(6))
        .unwrap();
    assert_ne!(next.sha256, first.sha256);
    assert_eq!(next.metadata.source.unwrap().entry_id, Some(6));
}

#[test]
fn store_generation_protects_snapshot_from_unrelated_new_correction() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let snapshot = store.write_snapshot("episode").unwrap();
    store
        .save_with_options(note("preference", "New explicit rule"), None, source(1))
        .unwrap();
    assert!(
        store
            .save_derived(
                note("episode", "built from older context"),
                &snapshot,
                source(2)
            )
            .is_err()
    );
    assert!(!dir.path().join("episode.md").exists());
}

#[test]
fn missing_ledger_or_unregistered_copy_is_rejected_without_importing() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let mut options = source(1);
    options.scope = Some(MemoryScope::Session {
        session_id: 42,
        instance_id: "runtime-a".into(),
    });
    let task = store
        .save_with_options(note("task", "Private session state"), None, options)
        .unwrap();
    let renamed = fs::read_to_string(&task.path)
        .unwrap()
        .replace("name: \"task\"", "name: \"renamed\"");
    fs::write(dir.path().join("renamed.md"), renamed).unwrap();
    assert!(store.get("renamed").is_err());
    assert!(store.write_snapshot("renamed").is_err());
    fs::remove_file(store.metadata_path()).unwrap();
    assert!(store.get("task").is_err());
    assert!(
        store
            .save_with_options(
                note("task", "Cannot import by overwriting"),
                Some(&task.sha256),
                MemoryWriteOptions::default()
            )
            .is_err()
    );
    assert!(
        fs::read_to_string(&task.path)
            .unwrap()
            .contains("Private session state")
    );
    assert!(!store.metadata_path().exists());
}

#[test]
fn checked_batch_delete_rejects_one_stale_revision_without_partial_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let a = store
        .save_with_options(note("a", "A"), None, MemoryWriteOptions::default())
        .unwrap();
    let b = store
        .save_with_options(note("b", "B"), None, MemoryWriteOptions::default())
        .unwrap();
    let index = store.index_lines().unwrap();
    let result = store.forget_many_checked(&[
        MemoryRevision {
            name: "a".into(),
            sha256: a.sha256.clone(),
        },
        MemoryRevision {
            name: "b".into(),
            sha256: "stale".into(),
        },
    ]);
    assert!(result.is_err());
    assert_eq!(store.get("a").unwrap().sha256, a.sha256);
    assert_eq!(store.get("b").unwrap().sha256, b.sha256);
    assert_eq!(store.index_lines().unwrap(), index);
    store
        .forget_many_checked(&[
            MemoryRevision {
                name: "a".into(),
                sha256: a.sha256,
            },
            MemoryRevision {
                name: "b".into(),
                sha256: b.sha256,
            },
        ])
        .unwrap();
    assert!(store.list().unwrap().is_empty());
    assert!(store.index_lines().unwrap().is_empty());
    assert!(store.write_snapshot("a").unwrap().is_tombstoned);
    assert!(store.write_snapshot("b").unwrap().is_tombstoned);
}

#[test]
fn metadata_corruption_fails_closed_without_overwriting_documents() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let first = store
        .save_with_options(
            note("decision", "Current fact"),
            None,
            MemoryWriteOptions::default(),
        )
        .unwrap();
    fs::write(store.metadata_path(), "not json").unwrap();
    assert!(store.get("decision").is_err());
    assert!(
        store
            .save_with_options(
                note("decision", "Wrong fact"),
                Some(&first.sha256),
                MemoryWriteOptions::default()
            )
            .is_err()
    );
    assert!(
        fs::read_to_string(first.path)
            .unwrap()
            .contains("Current fact")
    );
}

#[test]
fn filtered_listing_does_not_read_denied_records() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    store
        .save_with_options(
            note("allowed", "Visible fact"),
            None,
            MemoryWriteOptions::default(),
        )
        .unwrap();
    fs::write(dir.path().join("denied.md"), [0xff, 0xfe]).unwrap();
    let records = store
        .list_filtered(|path| path.file_name().unwrap() != "denied.md")
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].file_name, "allowed.md");
}

#[test]
fn invalid_status_or_cross_scope_supersession_cannot_publish() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let mut options = source(1);
    options.scope = Some(MemoryScope::Session {
        session_id: 42,
        instance_id: "runtime-a".into(),
    });
    let scoped = store
        .save_with_options(note("session-note", "Private task context"), None, options)
        .unwrap();
    let mut replacement = source(2);
    replacement.supersedes.push(MemoryRevision {
        name: "session-note".into(),
        sha256: scoped.sha256.clone(),
    });
    assert!(
        store
            .save_with_options(
                note("workspace-note", "Cannot replace scope"),
                None,
                replacement
            )
            .is_err()
    );
    let invalid = MemoryWriteOptions {
        status: Some(MemoryStatus::Superseded),
        ..MemoryWriteOptions::default()
    };
    assert!(
        store
            .save_with_options(note("invalid", "Missing replacement"), None, invalid)
            .is_err()
    );
    assert!(
        store
            .get("session-note")
            .unwrap()
            .is_active_in_context(Some(42), Some("runtime-a"))
    );
}

#[test]
fn instance_scoped_tasks_do_not_leak_across_database_resets() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let mut options = source(1);
    options.source.as_mut().unwrap().instance_id = "runtime-a".into();
    options.scope = Some(MemoryScope::Session {
        session_id: 42,
        instance_id: "runtime-a".into(),
    });
    let task = store
        .save_with_options(note("task", "State in runtime A"), None, options)
        .unwrap();
    assert!(task.is_active_in_context(Some(42), Some("runtime-a")));
    assert!(!task.is_active_in_context(Some(42), Some("runtime-b")));
    assert!(!task.is_active_in_context(Some(42), None));
    assert!(!task.is_active_in_context(Some(43), Some("runtime-a")));
    assert!(serde_json::from_str::<MemoryScope>(r#"{"kind":"session","session_id":42}"#).is_err());
    assert!(serde_json::from_str::<MemorySource>(r#"{"session_id":42,"entry_id":1}"#).is_err());
    assert!(
        serde_json::from_str::<MemorySource>(r#"{"instance_id":"runtime-a","entry_id":1}"#)
            .is_err()
    );
}

#[test]
fn derived_sources_cannot_move_to_another_runtime_instance() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let mut options = source(1);
    options.source.as_mut().unwrap().instance_id = "runtime-a".into();
    let snapshot = store.write_snapshot("episode").unwrap();
    let first = store
        .save_derived(note("episode", "Original instance"), &snapshot, options)
        .unwrap();
    let mut changed = source(2);
    changed.source.as_mut().unwrap().instance_id = "runtime-b".into();
    let snapshot = store.write_snapshot("episode").unwrap();
    assert!(
        store
            .save_derived(note("episode", "Wrong instance"), &snapshot, changed)
            .is_err()
    );
    assert_eq!(store.get("episode").unwrap().sha256, first.sha256);
}

#[test]
fn authorized_recovery_restores_document_index_and_lifecycle_state_together() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let original = store
        .save_with_options(note("decision", "Committed fact"), None, source(1))
        .unwrap();
    let old_doc = fs::read_to_string(&original.path).unwrap();
    let old_index = fs::read_to_string(dir.path().join(ENTRYPOINT_NAME)).unwrap();
    let old_state = fs::read_to_string(store.metadata_path()).unwrap();
    let journal = serde_json::json!({
        "schema_version": 1,
        "entries": [
            {"name": "decision.md", "previous": old_doc},
            {"name": ENTRYPOINT_NAME, "previous": old_index},
            {"name": state::STATE_NAME, "previous": old_state}
        ]
    });
    fs::write(
        dir.path().join(transaction::JOURNAL_NAME),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();
    fs::write(&original.path, "uncommitted change").unwrap();
    fs::write(dir.path().join(ENTRYPOINT_NAME), "uncommitted index").unwrap();
    fs::write(store.metadata_path(), "partial state").unwrap();
    assert!(store.get("decision").is_err());
    let reopened = MemoryStore::at(dir.path());
    assert!(reopened.ensure_index().is_err());
    assert!(
        reopened
            .save_with_options(
                note("new", "must not recover implicitly"),
                None,
                MemoryWriteOptions::default()
            )
            .is_err()
    );
    let approved = reopened.pending_recovery_paths().unwrap();
    assert!(approved.contains(&original.path));
    assert!(approved.contains(&store.metadata_path()));
    assert!(
        reopened
            .recover_pending(|path| approved.iter().any(|allowed| allowed == path))
            .unwrap()
    );
    assert!(reopened.pending_recovery_paths().unwrap().is_empty());
    assert!(!reopened.recover_pending(|_| false).unwrap());
    reopened.ensure_index().unwrap();
    let recovered = reopened.get("decision").unwrap();
    assert_eq!(recovered.sha256, original.sha256);
    assert_eq!(recovered.metadata, original.metadata);
    assert!(
        reopened
            .index_lines()
            .unwrap()
            .join("\n")
            .contains("(decision.md)")
    );
}

#[test]
fn forgotten_sources_suppress_derived_notes_across_names_and_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let mut options = source(1);
    options.source.as_mut().unwrap().instance_id = "runtime-a".into();
    let forgotten_source = options.source.clone().unwrap();
    let fact = store
        .save_with_options(note("private-fact", "User fact"), None, options.clone())
        .unwrap();
    let snapshot = store.write_snapshot("episode").unwrap();
    store
        .save_derived(
            note("episode", "A task learned the user fact"),
            &snapshot,
            options.clone(),
        )
        .unwrap();
    let before_forget = store.write_snapshot("renamed-episode").unwrap();

    store.forget_checked("private-fact", &fact.sha256).unwrap();
    let hidden = store.get("episode").unwrap();
    assert!(hidden.metadata.source_suppressed);
    assert!(!hidden.is_active_in_context(Some(42), Some("runtime-a")));
    assert_eq!(hidden.metadata.status, MemoryStatus::Archived);
    assert!(
        !store
            .index_lines()
            .unwrap()
            .join("\n")
            .contains("(episode.md)")
    );
    assert!(
        store
            .save_derived(
                note("renamed-episode", "Stale fact"),
                &before_forget,
                options.clone()
            )
            .is_err()
    );
    let fresh = store.write_snapshot("new-name").unwrap();
    assert!(
        store
            .save_derived(
                note("new-name", "Fact from a later extraction"),
                &fresh,
                options.clone()
            )
            .is_err()
    );

    let reopened = MemoryStore::at(dir.path());
    assert!(reopened.source_is_suppressed(&forgotten_source).unwrap());
    assert!(reopened.get("episode").unwrap().metadata.source_suppressed);
    let explicit = reopened
        .save_with_options(
            note("private-fact", "User deliberately restored it"),
            None,
            options.clone(),
        )
        .unwrap();
    assert!(!explicit.metadata.source_suppressed);
    assert!(explicit.is_active_in_context(Some(42), Some("runtime-a")));
    assert!(reopened.source_is_suppressed(&forgotten_source).unwrap());

    let mut other_instance = options;
    other_instance.source.as_mut().unwrap().instance_id = "runtime-b".into();
    let fresh = reopened.write_snapshot("other-instance").unwrap();
    assert!(
        reopened
            .save_derived(
                note("other-instance", "Different source conversation"),
                &fresh,
                other_instance
            )
            .unwrap()
            .is_active_in_context(Some(42), Some("runtime-a"))
    );
}

#[test]
fn externally_touching_suppressed_derived_file_does_not_restore_model_visibility() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let mut options = source(1);
    options.source.as_mut().unwrap().instance_id = "runtime-a".into();
    let fact = store
        .save_with_options(note("fact", "Forget this"), None, options.clone())
        .unwrap();
    let snapshot = store.write_snapshot("episode").unwrap();
    let episode = store
        .save_derived(
            note("episode", "Summary containing the fact"),
            &snapshot,
            options,
        )
        .unwrap();
    store.forget_checked("fact", &fact.sha256).unwrap();
    let raw = fs::read_to_string(&episode.path).unwrap();
    fs::write(&episode.path, format!("{raw}\n")).unwrap();
    let external = MemoryStore::at(dir.path()).get("episode").unwrap();
    assert_eq!(external.metadata.origin, MemoryOrigin::External);
    assert!(external.metadata.source.is_none());
    assert!(!external.metadata.provenance_verified);
    assert!(external.metadata.source_suppressed);
    assert!(!external.is_active_in_context(Some(42), Some("runtime-a")));

    let reviewed = store
        .save_with_options(
            note("episode", "A user-reviewed replacement"),
            Some(&external.sha256),
            MemoryWriteOptions {
                status: Some(MemoryStatus::Active),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(reviewed.metadata.origin, MemoryOrigin::Explicit);
    assert!(!store.get("episode").unwrap().metadata.source_suppressed);
    assert!(reviewed.is_active_in_context(Some(42), Some("runtime-a")));
}

#[test]
fn explicit_edits_preserve_prior_sources_for_durable_forget() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let mut a = source(1);
    a.source.as_mut().unwrap().instance_id = "runtime-a".into();
    let first = store
        .save_with_options(note("fact", "Original fact"), None, a.clone())
        .unwrap();
    let snapshot = store.write_snapshot("episode-a").unwrap();
    store
        .save_derived(
            note("episode-a", "Original source summary"),
            &snapshot,
            a.clone(),
        )
        .unwrap();
    let mut b = a.clone();
    b.source.as_mut().unwrap().session_id = 43;
    let edited = store
        .save_with_options(
            note("fact", "Corrected from another session"),
            Some(&first.sha256),
            b.clone(),
        )
        .unwrap();
    let snapshot = store.write_snapshot("episode-b").unwrap();
    store
        .save_derived(
            note("episode-b", "Later source summary"),
            &snapshot,
            b.clone(),
        )
        .unwrap();
    // Operator edits intentionally clear the current text's source attribution.
    let manual = store
        .save_with_options(
            note("fact", "User edited this in the UI"),
            Some(&edited.sha256),
            MemoryWriteOptions::default(),
        )
        .unwrap();
    assert!(manual.metadata.source.is_none());
    assert!(!manual.metadata.provenance_verified);
    store
        .forget_many_checked(&[MemoryRevision {
            name: "fact".into(),
            sha256: manual.sha256,
        }])
        .unwrap();
    let reopened = MemoryStore::at(dir.path());
    for name in ["episode-a", "episode-b"] {
        assert!(reopened.get(name).unwrap().metadata.source_suppressed);
    }
    for options in [a, b] {
        assert!(
            reopened
                .source_is_suppressed(options.source.as_ref().unwrap())
                .unwrap()
        );
        let snapshot = reopened.write_snapshot("new-name").unwrap();
        assert!(
            reopened
                .save_derived(note("new-name", "Forgotten claim"), &snapshot, options)
                .is_err()
        );
    }
}

#[test]
fn pending_recovery_requires_every_current_target_and_never_creates_files_when_absent() {
    let parent = tempfile::tempdir().unwrap();
    let absent_dir = parent.path().join("absent");
    let absent = MemoryStore::at(&absent_dir);
    assert!(absent.pending_recovery_paths().unwrap().is_empty());
    assert!(!absent.recover_pending(|_| false).unwrap());
    assert!(!absent_dir.exists());

    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::at(dir.path());
    let note_path = dir.path().join("note.md");
    let journal_path = dir.path().join(transaction::JOURNAL_NAME);
    let journal = serde_json::json!({"schema_version":1,"entries":[{"name":"note.md","previous":"committed"}]});
    fs::write(&note_path, "partial").unwrap();
    fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
    let approved = store.pending_recovery_paths().unwrap();
    assert!(!dir.path().join(transaction::LOCK_NAME).exists());
    let denied = store.recover_pending(|path| path != note_path).unwrap_err();
    assert!(
        matches!(denied, MemoryError::Io(ref error) if error.kind() == io::ErrorKind::PermissionDenied)
    );
    assert_eq!(fs::read_to_string(&note_path).unwrap(), "partial");
    assert!(journal_path.exists());
    assert!(!dir.path().join(transaction::LOCK_NAME).exists());

    // A changed recovery inventory must not inherit an earlier authorization.
    let added = dir.path().join("unapproved.md");
    fs::write(&added, "partial secret").unwrap();
    let changed = serde_json::json!({"schema_version":1,"entries":[
        {"name":"note.md","previous":"committed"},
        {"name":"unapproved.md","previous":"private"}
    ]});
    fs::write(&journal_path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(
        store
            .recover_pending(|path| approved.iter().any(|allowed| allowed == path))
            .is_err()
    );
    assert_eq!(fs::read_to_string(&added).unwrap(), "partial secret");
    assert_eq!(fs::read_to_string(&note_path).unwrap(), "partial");
    assert!(store.recover_pending(|_| true).unwrap());
    assert_eq!(fs::read_to_string(&added).unwrap(), "private");
    assert_eq!(fs::read_to_string(&note_path).unwrap(), "committed");
}

#[cfg(unix)]
#[test]
fn lifecycle_and_lock_symlinks_are_never_followed() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), "private bytes").unwrap();
    let store = MemoryStore::at(dir.path());
    symlink(outside.path(), store.metadata_path()).unwrap();
    assert!(store.list().is_err());
    assert!(
        store
            .save_with_options(note("note", "bad"), None, MemoryWriteOptions::default())
            .is_err()
    );
    assert_eq!(fs::read_to_string(outside.path()).unwrap(), "private bytes");
    fs::remove_file(store.metadata_path()).unwrap();
    fs::remove_file(dir.path().join(transaction::LOCK_NAME)).unwrap();
    symlink(outside.path(), dir.path().join(transaction::LOCK_NAME)).unwrap();
    assert!(store.ensure_index().is_err());
    assert_eq!(fs::read_to_string(outside.path()).unwrap(), "private bytes");
}
