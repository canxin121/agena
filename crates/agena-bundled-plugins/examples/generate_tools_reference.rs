//! Regenerate `docs/generated/tools-reference.md` from the real bundled
//! plugin manifests:
//!
//! ```bash
//! cargo run -p agena-bundled-plugins --example generate_tools_reference \
//!   > docs/generated/tools-reference.md
//! ```
//!
//! The same document is embedded into rustdoc via `include_str!`, so `cargo
//! doc` renders every bundled tool definition together with its detailed help
//! text. A CI drift test compares the committed file against this output.

fn main() {
    print!(
        "{}",
        agena_bundled_plugins::bundled_tools_markdown_reference()
    );
}
