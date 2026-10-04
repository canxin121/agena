use super::*;
use serde_json::json;

fn input(value: serde_json::Value) -> DocumentInput {
    serde_json::from_value(value).unwrap()
}

#[test]
fn projected_search_counts_lines_and_marks_exact_boundaries() {
    let args = input(json!({"path":"x.pdf","pattern":"a.b","max_lines":1}));
    let regex = matcher(&args).unwrap();
    let exact = project("other\na.b\naxb", regex.as_ref(), &args);
    assert_eq!(exact.matching_lines, 1);
    assert!(!exact.truncated);
    assert_eq!(exact.lines[0].line, 2);
    let more = project("a.b\na.b", regex.as_ref(), &args);
    assert_eq!(more.matching_lines, 2);
    assert!(more.truncated);
    let args = input(
        json!({"path":"x.pdf","pattern":"a.b","fixed_strings":false,"ignore_case":true,"start_line":2}),
    );
    let result = project("a.b\nAXB\na.b", matcher(&args).unwrap().as_ref(), &args);
    assert_eq!(result.matching_lines, 2);
    assert_eq!(result.lines[0].line, 2);
}

#[test]
fn projection_preserves_unicode_and_bounds_all_escaped_records() {
    let args = input(json!({"path":"x.pdf","max_lines":500}));
    let source = format!("{}\n", "中文\t".repeat(1200)).repeat(1000);
    let result = project(&source, None, &args);
    assert!(result.truncated);
    assert_eq!(result.matching_lines, 1000);
    assert!(serde_json::to_vec(&result.lines).unwrap().len() <= MAX_RECORDS + 2);
    assert!(
        result
            .lines
            .iter()
            .all(|line| line.text_truncated && line.text.len() <= 4096)
    );
}

#[test]
fn source_snapshot_and_backend_constraints_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("test.docx"), b"source snapshot").unwrap();
    let source = prepare(root.path(), "test.docx").unwrap();
    std::fs::write(root.path().join("test.docx"), b"new bytes").unwrap();
    assert_eq!(std::fs::read(&source.path).unwrap(), b"source snapshot");
    assert_eq!(source.sha256, sha256_bytes(b"source snapshot"));
    assert!(command(root.path(), &source, Backend::Pdftotext).is_err());
    assert!(prepare(root.path(), "remote.wav").is_err());
    assert!(
        matcher(&input(
            json!({"path":"x.pdf","pattern":"[","fixed_strings":false})
        ))
        .is_err()
    );
}
