use super::*;
use serde_json::json;

fn request(path: &Path, pattern: &str) -> StructuralSearchRequest {
    StructuralSearchRequest {
        path: path.to_owned(),
        pattern: pattern.into(),
        rule: None,
        language: Some(CodeLanguage::Javascript),
        limit: None,
    }
}

#[test]
fn structured_rules_filter_ancestors_and_limits_require_an_extra_match() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("sample.js");
    std::fs::write(
        &file,
        "console.log('outside'); function test() { console.log('one'); console.log('two'); }",
    )
    .unwrap();
    let mut query = request(&file, "");
    query.rule = Some(
        json!({"all":[{"pattern":"console.log($A)"}, {"inside":{"kind":"function_declaration","stopBy":"end"}}]}),
    );
    query.limit = Some(2);
    let exact = search_ast(root.path(), query.clone()).unwrap();
    assert_eq!(exact.matches.len(), 2);
    assert!(!exact.truncated);
    assert!(
        !exact
            .matches
            .iter()
            .any(|item| item.text.contains("outside"))
    );
    query.limit = Some(1);
    let partial = search_ast(root.path(), query).unwrap();
    assert_eq!(partial.matches.len(), 1);
    assert!(partial.truncated);
    assert!(
        partial
            .truncation_reason
            .unwrap()
            .contains("additional AST matches")
    );
}

#[test]
fn rewrite_preserves_unicode_crlf_and_only_changes_structural_matches() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("sample.js");
    let original = "// console.log(fake) 中文\r\nconsole.log('你好');\r\nconsole.log('二');\r\n";
    std::fs::write(&file, original).unwrap();
    let plan = plan_rewrite(
        root.path(),
        request(&file, "console.log($A)"),
        "logger.info($A)",
    )
    .unwrap();
    assert_eq!(plan.replacements, 2);
    assert_eq!(plan.original, original);
    assert_eq!(
        plan.updated,
        "// console.log(fake) 中文\r\nlogger.info('你好');\r\nlogger.info('二');\r\n"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
}

#[test]
fn rewrite_refuses_partial_ambiguous_or_invalid_edits() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("sample.js");
    std::fs::write(&file, "console.log(1); console.log(2);").unwrap();
    let mut query = request(&file, "console.log($A)");
    query.limit = Some(1);
    assert!(
        plan_rewrite(root.path(), query, "logger.info($A)")
            .unwrap_err()
            .to_string()
            .contains("limit")
    );
    assert!(plan_rewrite(root.path(), request(&file, "console.log($A)"), "$UNDEFINED").is_err());
    assert!(
        plan_rewrite(root.path(), request(&file, "console.log($A)"), "function (")
            .unwrap_err()
            .to_string()
            .contains("parse errors")
    );
    std::fs::write(&file, "foo(foo(1));").unwrap();
    assert!(
        plan_rewrite(root.path(), request(&file, "foo($A)"), "bar($A)")
            .unwrap_err()
            .to_string()
            .contains("overlapping")
    );
    std::fs::write(&file, "function ( { console.log(1);").unwrap();
    assert!(
        plan_rewrite(
            root.path(),
            request(&file, "console.log($A)"),
            "logger.info($A)"
        )
        .unwrap_err()
        .to_string()
        .contains("parse errors")
    );
}

#[test]
fn search_limits_across_files_and_rule_bounds_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    for name in ["a.js", "b.js"] {
        std::fs::write(root.path().join(name), "console.log(1);").unwrap();
    }
    let mut query = request(root.path(), "console.log($A)");
    query.limit = Some(1);
    let result = search_ast(root.path(), query.clone()).unwrap();
    assert!(result.truncated);
    assert_eq!(result.matches[0].path, "a.js");
    query.limit = Some(0);
    assert!(search_ast(root.path(), query).is_err());
    let mut query = request(root.path(), "");
    let mut rule = json!({"kind":"call_expression"});
    for _ in 0..20 {
        rule = json!({"not":rule});
    }
    query.rule = Some(rule);
    assert!(
        search_ast(root.path(), query)
            .unwrap_err()
            .to_string()
            .contains("16 levels")
    );
}

#[test]
fn broad_pattern_compatibility_and_large_tree_previews_are_bounded() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("sample.js");
    std::fs::write(&file, "const x = 1;\n").unwrap();
    assert!(
        !search_ast(root.path(), request(&file, "$A"))
            .unwrap()
            .matches
            .is_empty()
    );
    std::fs::write(
        &file,
        "function demo() {\n".to_owned() + &"console.log(1);\n".repeat(1000) + "}\n",
    )
    .unwrap();
    let result = syntax_tree(
        root.path(),
        SyntaxTreeRequest {
            path: file,
            language: None,
            max_depth: Some(6),
        },
    )
    .unwrap();
    fn count(node: &SyntaxNodeView) -> usize {
        1 + node.children.iter().map(count).sum::<usize>()
    }
    assert!(result.truncated);
    assert!(count(&result.tree) <= 512);
}

#[test]
fn failed_utf8_reads_consume_the_shared_content_budget() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("sample.js");
    std::fs::write(&file, [0xff, 0xfe, 0xfd]).unwrap();
    let mut remaining = 4;
    assert!(matches!(
        read_source_with_budget(&file, &mut remaining),
        Err(CodeSearchError::InvalidParameters(_))
    ));
    assert_eq!(remaining, 1);
    assert!(matches!(
        read_source_with_budget(&file, &mut remaining),
        Err(CodeSearchError::SourceBudget)
    ));
    assert_eq!(
        remaining, 1,
        "an oversized reservation must fail before reading"
    );
}
