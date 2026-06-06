#[test]
fn plugin_layer_macro_compile_failures_are_clear() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/plugin_macro/*.rs");
}
