use super::*;
use serde_json::json;

#[test]
fn preview_apply_and_stale_revision_preserve_the_file_contract() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file.js");
    let original = "console.log('你好');\n";
    std::fs::write(&file, original).unwrap();
    let args =
        json!({"path":"file.js", "pattern":"console.log($A)", "replacement":"logger.info($A)"});
    let preview =
        invoke_rewrite(root.path(), serde_json::from_value(args.clone()).unwrap()).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    assert!(!preview.metadata.contains_key("agena.effect"));
    let payload = preview.payload.unwrap();
    assert_eq!(payload["applied"], false);
    assert_eq!(payload["replacements"], 1);
    assert!(payload["diff"].as_str().unwrap().contains("+logger.info"));
    let mut apply = args.clone();
    apply["apply"] = json!(true);
    assert!(invoke_rewrite(root.path(), serde_json::from_value(apply.clone()).unwrap()).is_err());
    apply["expected_sha256"] = payload["before_sha256"].clone();
    std::fs::write(&file, "// external edit\nconsole.log('你好');\n").unwrap();
    assert!(
        invoke_rewrite(root.path(), serde_json::from_value(apply.clone()).unwrap())
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .starts_with("// external edit")
    );
    std::fs::write(&file, original).unwrap();
    let applied = invoke_rewrite(root.path(), serde_json::from_value(apply).unwrap()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "logger.info('你好');\n"
    );
    assert_eq!(
        applied.metadata.get("agena.effect").unwrap(),
        "file_changes"
    );
    assert_eq!(
        applied.payload.unwrap()["after_sha256"],
        payload["after_sha256"]
    );
}
