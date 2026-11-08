use crate::{DEFAULT_EXCERPT_CHARS, MemoryIndexError, MemorySearchDocument};

fn document(id: &str, description: &str, body: &str) -> MemorySearchDocument {
    MemorySearchDocument::new(
        id.to_owned(),
        id.to_owned(),
        description.to_owned(),
        Some("project".to_owned()),
        body.to_owned(),
        format!("/memory/{id}.md"),
    )
}

fn examples() -> Vec<MemorySearchDocument> {
    vec![
        document(
            "database-choice",
            "数据库选型",
            "项目采用 PostgreSQL 存储业务数据，不使用 MySQL。",
        ),
        document("weather", "天气偏好", "用户希望报告包含北京天气。"),
        document("roadmap", "本周任务", "本周优化 agent 的记忆检索体验。"),
        document(
            "english-database",
            "database selection",
            "Use PostgreSQL for the application database and avoid MySQL.",
        ),
    ]
}

macro_rules! backend_contract {
    ($module:ident, $index:ty) => {
        mod $module {
            use super::*;

            fn fixture() -> (tempfile::TempDir, $index) {
                let temp = tempfile::tempdir().unwrap();
                let index = <$index>::for_test(temp.path().join("index"));
                (temp, index)
            }

            #[test]
            fn chinese_natural_sentence_retrieves_its_topic() {
                let (_temp, index) = fixture();
                index.replace_documents(&examples()).unwrap();
                for query in ["数据库", "数据库选型", "我们之前数据库选型是什么", "我们之前数据库用什么"] {
                    let hits = index.search_with_details(query, 3).unwrap();
                    assert_eq!(hits.first().map(|hit| hit.document.id.as_str()), Some("database-choice"), "query: {query}");
                    assert!(hits[0].score > 0.0);
                    assert!(!hits[0].matched_terms.is_empty());
                }
                let hits = index.search("今天北京天气怎么样", 3).unwrap();
                assert_eq!(hits.first().map(|hit| hit.id.as_str()), Some("weather"));
            }

            #[test]
            fn ascii_words_do_not_match_arbitrary_substrings() {
                let (_temp, index) = fixture();
                index.replace_documents(&[document("weather", "天气偏好", "The forecast includes Beijing weather and concatenation examples.")]).unwrap();
                for query in ["What storage engine did we choose?", "cat", "we", "unrelated-symbol-xyz"] {
                    assert!(index.search(query, 3).unwrap().is_empty(), "query: {query}");
                }
                assert_eq!(index.search("weather", 3).unwrap().len(), 1);
            }

            #[test]
            fn script_boundaries_and_code_identifiers_are_searchable() {
                let (_temp, index) = fixture();
                index.replace_documents(&[
                    document("storage", "数据库选型", "数据库PostgreSQL存储业务数据。Implementation: crates/agena-storage/src/memory_store.rs; MemoryStore::save_checked."),
                    document("other", "Other code", "Implementation: crates/agena-storage/src/other.rs; MemoryStore::delete_checked."),
                ]).unwrap();
                for query in ["PostgreSQL", "数据库PostgreSQL", "MemoryStore::save_checked", "检查MemoryStore::save_checked的实现", "crates/agena-storage/src/memory_store.rs"] {
                    let hits = index.search_with_details(query, 3).unwrap();
                    assert_eq!(hits.first().map(|hit| hit.document.id.as_str()), Some("storage"), "query: {query}");
                }
            }

            #[test]
            fn arbitrary_query_punctuation_is_literal_and_bounded() {
                let (_temp, index) = fixture();
                assert!(index.search("anything", 3).unwrap().is_empty());
                index.replace_documents(&examples()).unwrap();
                for query in ["", "   ", "() [] {} + - * ? \" ' : \\ /", "AND OR NOT"] {
                    assert!(index.search(query, 3).unwrap().is_empty(), "query: {query}");
                }
                let hits = index.search("name:\"PostgreSQL +( [", 3).unwrap();
                assert!(hits.iter().any(|hit| hit.id == "database-choice"));
                assert!(index.search("PostgreSQL", 0).unwrap().is_empty());
                let long_query = format!("{} PostgreSQL", "普通上下文。".repeat(2000));
                assert!(index.search(&long_query, 3).unwrap().iter().any(|hit| hit.id == "database-choice"));
            }

            #[test]
            fn paths_are_searchable_without_body_mentions_and_preserve_source_casing() {
                let (_temp, index) = fixture();
                let mut target = document("path-target", "Saved decision", "Unrelated record body.");
                target.path = "/Memory/Reports/Foo_İSTANBUL.rs".to_owned();
                let mut other = document("path-other", "Saved decision", "Unrelated record body.");
                other.path = "/Memory/Reports/Unrelated.md".to_owned();
                let mut cjk = document("path-cjk", "Saved decision", "Unrelated record body.");
                cjk.path = "/记忆/数据库选型.md".to_owned();
                index.replace_documents(&[target.clone(), other.clone(), cjk.clone()]).unwrap();
                for query in ["/Memory/Reports/Foo_İSTANBUL.rs", "/memory/reports/foo_i\u{307}stanbul.rs", "Foo_İSTANBUL.rs", "İSTANBUL"] {
                    let hits = index.search_with_details(query, 3).unwrap();
                    assert_eq!(hits.len(), 1, "query: {query}");
                    assert_eq!(hits[0].document, target, "query: {query}");
                    assert!(!hits[0].matched_terms.is_empty(), "query: {query}");
                }
                assert_eq!(index.search("我们之前数据库选型是什么", 3).unwrap()[0].id, "path-cjk");
                target.path = "/Memory/Renamed/Bar.rs".to_owned();
                index.replace_documents(&[target.clone(), other.clone(), cjk.clone()]).unwrap();
                assert!(index.search("/memory/reports/foo_i\u{307}stanbul.rs", 3).unwrap().is_empty());
                assert_eq!(index.search("/memory/renamed/bar.rs", 3).unwrap()[0], target);
                index.replace_documents(&[other, cjk]).unwrap();
                assert!(index.search("/memory/renamed/bar.rs", 3).unwrap().is_empty());
            }

            #[test]
            fn a_late_match_returns_its_passage_and_source_lines() {
                let (_temp, index) = fixture();
                let prefix = "Unrelated historical introduction.\n\n".repeat(150);
                let body = format!("{prefix}Verified recovery: run agena repair --reindex for E9A31.\nThe repair completed successfully.\n\nLater unrelated notes.");
                index.replace_documents(&[document("recovery", "Recovered indexing", &body)]).unwrap();
                let hits = index.search_with_details("How did we resolve E9A31?", 3).unwrap();
                assert_eq!(hits.len(), 1);
                let excerpt = &hits[0].excerpt;
                assert!(excerpt.text.contains("E9A31"), "{excerpt:?}");
                assert!(excerpt.text.contains("agena repair --reindex"));
                assert!(excerpt.text.chars().count() <= DEFAULT_EXCERPT_CHARS);
                assert!(body.contains(&excerpt.text));
                assert_eq!(excerpt.start_line, 301);
                assert_eq!(excerpt.end_line, 302);
                assert!(hits[0].matched_terms.contains(&"e9a31".to_owned()));
            }

            #[test]
            fn unicode_folding_matches_the_index_without_losing_source_text() {
                let (_temp, index) = fixture();
                index.replace_documents(&[document("unicode-case", "Case folding", "İSTANBUL ΟΣ CAFÉ")]).unwrap();
                for query in ["İSTANBUL", "i\u{307}stanbul", "ΟΣ", "CAFÉ", "café"] {
                    let hits = index.search_with_details(query, 3).unwrap();
                    assert_eq!(hits.len(), 1, "query: {query}");
                    assert!(hits[0].excerpt.text.contains("İSTANBUL"));
                }
            }

            #[test]
            fn updates_deletions_and_reopen_preserve_only_the_current_set() {
                let (temp, index) = fixture();
                index.replace_documents(&[
                    document("decision", "Build decision", "Use old_command_xyz for the build."),
                    document("private", "Private note", "secret_marker_xyz"),
                ]).unwrap();
                let reopened = <$index>::for_test(temp.path().join("index"));
                assert_eq!(reopened.search("old_command_xyz", 3).unwrap().len(), 1);
                reopened.replace_documents(&[document("decision", "Build decision", "Use replacement_marker_abc for the build.")]).unwrap();
                assert!(reopened.search("old_command_xyz", 3).unwrap().is_empty());
                assert!(reopened.search("secret_marker_xyz", 3).unwrap().is_empty());
                let hits = reopened.search("replacement_marker_abc", 3).unwrap();
                assert_eq!(hits.len(), 1);
                assert_eq!(hits[0].id, "decision");
                reopened.replace_documents(&[]).unwrap();
                assert!(reopened.search("replacement_marker_abc", 3).unwrap().is_empty());
            }

            #[test]
            fn invalid_duplicate_ids_leave_the_existing_index_unchanged() {
                let (_temp, index) = fixture();
                let original = document("decision", "Decision", "confirmed_marker_xyz");
                index.replace_documents(std::slice::from_ref(&original)).unwrap();
                let error = index.replace_documents(&[original.clone(), document("decision", "Conflict", "incorrect_marker_abc")]).unwrap_err();
                assert!(matches!(error, MemoryIndexError::InvalidDocuments(_)));
                let hits = index.search("confirmed_marker_xyz", 3).unwrap();
                assert_eq!(hits.len(), 1);
                assert_eq!(hits[0].body, original.body);
                assert!(index.search("incorrect_marker_abc", 3).unwrap().is_empty());
            }
        }
    };
}

#[cfg(target_endian = "little")]
backend_contract!(tantivy_contract, crate::tantivy_backend::MemoryIndex);
backend_contract!(portable_contract, crate::portable_backend::MemoryIndex);

#[test]
fn excerpts_preserve_unicode_and_respect_the_character_budget() {
    let body = "开头说明🔧。\n\n目标段落：数据库使用PostgreSQL，保留İSTANBUL案例与中文说明。\n\n后续说明。";
    let document = document("unicode", "Unicode", body);
    for budget in [0, 1, 7, 13, 40, 800] {
        let excerpt = document.excerpt("PostgreSQL 数据库", budget);
        assert!(excerpt.text.chars().count() <= budget);
        assert!(body.contains(&excerpt.text));
        if budget == 0 {
            assert_eq!((excerpt.start_line, excerpt.end_line), (0, 0));
        } else {
            assert!(excerpt.start_line >= 1 && excerpt.end_line >= excerpt.start_line);
        }
    }
    let excerpt = document.excerpt("İSTANBUL", 40);
    assert!(excerpt.text.contains("İSTANBUL"));
    assert_eq!((excerpt.start_line, excerpt.end_line), (3, 3));
    let prefix = document.excerpt("", 6);
    assert_eq!(prefix.text, body.chars().take(6).collect::<String>());
}

#[test]
fn a_small_excerpt_starts_at_the_relevant_line_after_a_long_background() {
    let body = format!(
        "{}\n数据库决策：使用 PostgreSQL，连接池大小为 16。\n{}",
        "无关背景资料。".repeat(400),
        "无关附录。".repeat(100)
    );
    let document = document("database-decision", "Database decision", &body);
    for budget in [24, 30, 32, 48, 256] {
        let excerpt = document.excerpt("数据库", budget);
        assert!(
            excerpt.text.starts_with("数据库决策："),
            "budget {budget}: {excerpt:?}"
        );
        assert!(
            excerpt.text.contains("PostgreSQL"),
            "budget {budget}: {excerpt:?}"
        );
        assert_eq!((excerpt.start_line, excerpt.end_line), (2, 2));
        assert!(excerpt.text.chars().count() <= budget);
    }
}

#[test]
fn an_oversized_single_line_still_yields_a_late_match() {
    let body = format!(
        "{}use PostgreSQL for persistence {}",
        "unrelated text ".repeat(500),
        "later text ".repeat(500)
    );
    let document = document("long-line", "Long line", &body);
    let excerpt = document.excerpt("PostgreSQL", 90);
    assert!(excerpt.text.contains("PostgreSQL"));
    assert!(excerpt.text.chars().count() <= 90);
    assert_eq!((excerpt.start_line, excerpt.end_line), (1, 1));
    assert!(body.contains(&excerpt.text));
}

#[cfg(target_endian = "little")]
#[test]
fn unchanged_sync_is_a_noop_and_changed_sync_keeps_untouched_segments() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("index");
    let index = crate::tantivy_backend::MemoryIndex::for_test(dir.clone());
    let mut documents = (0..20)
        .map(|n| {
            document(
                &format!("note-{n}"),
                "Note",
                &format!("original_unique_value_{n}"),
            )
        })
        .collect::<Vec<_>>();
    index.replace_documents(&documents).unwrap();
    let first = tantivy::Index::open_in_dir(&dir)
        .unwrap()
        .load_metas()
        .unwrap();
    documents.reverse();
    index.replace_documents(&documents).unwrap();
    let unchanged = tantivy::Index::open_in_dir(&dir)
        .unwrap()
        .load_metas()
        .unwrap();
    assert_eq!(first.opstamp, unchanged.opstamp);
    assert_eq!(first.payload, unchanged.payload);
    documents[0] = document("note-19", "Updated", "replacement_content");
    index.replace_documents(&documents).unwrap();
    let changed = tantivy::Index::open_in_dir(&dir)
        .unwrap()
        .load_metas()
        .unwrap();
    assert!(changed.opstamp > first.opstamp);
    assert!(
        changed
            .segments
            .iter()
            .any(|segment| first.segments.iter().any(|old| old.id() == segment.id())),
        "unchanged documents should retain their existing segment"
    );
    let reader = tantivy::Index::open_in_dir(&dir).unwrap().reader().unwrap();
    assert_eq!(reader.searcher().num_docs(), 20);
    assert_eq!(index.search("replacement_content", 3).unwrap().len(), 1);
}

#[cfg(target_endian = "little")]
#[test]
fn a_legacy_payload_is_migrated_once_before_incremental_updates() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("index");
    let index = crate::tantivy_backend::MemoryIndex::for_test(dir.clone());
    index.replace_documents(&examples()).unwrap();
    {
        let legacy = tantivy::Index::open_in_dir(&dir).unwrap();
        let mut writer = legacy
            .writer::<tantivy::TantivyDocument>(15_000_000)
            .unwrap();
        let mut commit = writer.prepare_commit().unwrap();
        commit.set_payload("agena-memory-v1:legacy-test-fingerprint");
        commit.commit().unwrap();
    }
    index.replace_documents(&examples()).unwrap();
    let metadata = tantivy::Index::open_in_dir(&dir)
        .unwrap()
        .load_metas()
        .unwrap();
    let payload: serde_json::Value =
        serde_json::from_str(metadata.payload.as_deref().unwrap()).unwrap();
    assert_eq!(payload["version"], 3);
    assert_eq!(payload["documents"].as_object().unwrap().len(), 4);
    assert_eq!(
        index.search("我们之前数据库选型是什么", 3).unwrap()[0].id,
        "database-choice"
    );
    index.replace_documents(&examples()).unwrap();
    assert_eq!(
        tantivy::Index::open_in_dir(&dir)
            .unwrap()
            .load_metas()
            .unwrap()
            .opstamp,
        metadata.opstamp
    );
}
