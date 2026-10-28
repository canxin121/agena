//! Generate the shared Web/TUI copy contract fixture.
fn main() {
    let value = serde_json::from_str(include_str!("../../../examples/inline-ui/reply.json"))
        .expect("fixture JSON");
    let document = agena_domain::InlineDocument::from_value(value).expect("valid inline document");
    println!("{}", document.to_markdown());
}
