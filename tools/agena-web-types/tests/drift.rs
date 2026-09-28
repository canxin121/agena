//! Fails when the committed TypeScript mirror no longer matches the backend.

use std::path::PathBuf;

#[test]
fn generated_typescript_mirrors_the_backend_state_types() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/agena-web/src/generated/agenaState.ts");
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    assert_eq!(
        committed,
        agena_web_types::module_source(),
        "the generated state types drifted; regenerate with `cargo run -p agena-web-types \
         > packages/agena-web/src/generated/agenaState.ts`"
    );
}
