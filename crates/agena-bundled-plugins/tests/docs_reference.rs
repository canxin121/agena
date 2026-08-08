use agena_bundled_plugins::bundled_tools_markdown_reference;

#[test]
fn tools_reference_matches_committed_markdown() {
    let reference_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("generated/tools-reference.md");
    let committed = std::fs::read_to_string(&reference_path).unwrap_or_else(|error| {
        panic!(
            "failed to read committed tools reference {}: {error}",
            reference_path.display()
        )
    });

    assert_eq!(
        committed,
        bundled_tools_markdown_reference(),
        "tools reference drifted; regenerate with \
         `cargo run -p agena-bundled-plugins --example generate_tools_reference \
         > crates/agena-bundled-plugins/generated/tools-reference.md`"
    );
}
