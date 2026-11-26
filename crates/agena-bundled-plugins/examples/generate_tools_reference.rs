//! Print a Markdown reference from the compiled bundled plugin manifests.
//!
//! ```bash
//! reference=crates/agena-bundled-plugins/generated/tools-reference.md
//! cargo run --locked -p agena-bundled-plugins --example generate_tools_reference > "$reference.tmp" &&
//!   mv "$reference.tmp" "$reference"
//! ```
//!
//! The build script uses the same reference to render the Rustdoc index and
//! individual plugin and tool pages. Library tests compare the committed file
//! with this output.

fn main() {
    print!(
        "{}",
        agena_bundled_plugins::bundled_tools_markdown_reference()
    );
}
