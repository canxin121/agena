use super::{MAX_CONTENT_REPLACE_PATHS, normalize_content_scope_paths};

#[tokio::test]
async fn mixed_directory_and_file_scopes_cannot_exceed_the_candidate_budget() {
    let fixture = tempfile::tempdir().unwrap();
    let many = fixture.path().join("many");
    std::fs::create_dir(&many).unwrap();
    for index in 0..MAX_CONTENT_REPLACE_PATHS - 1 {
        std::fs::write(many.join(index.to_string()), b"").unwrap();
    }
    for name in ["extra-one", "extra-two"] {
        std::fs::write(fixture.path().join(name), b"").unwrap();
    }
    let scope = vec![
        "many".to_owned(),
        "extra-one".to_owned(),
        "extra-two".to_owned(),
    ];
    let (files, truncated) = normalize_content_scope_paths(fixture.path(), &scope, true, false)
        .await
        .unwrap();
    assert_eq!(files.len(), MAX_CONTENT_REPLACE_PATHS);
    assert!(truncated);
}
