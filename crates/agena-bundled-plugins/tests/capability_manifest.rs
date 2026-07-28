use agena_bundled_plugins::bundled_capability_manifest;

#[test]
fn bundled_manifest_separates_gateway_and_execution_tools() {
    let manifest = bundled_capability_manifest();
    let tool_count = manifest
        .plugins
        .iter()
        .map(|plugin| plugin.tools.len())
        .sum::<usize>();
    let gateway_count = manifest
        .plugins
        .iter()
        .flat_map(|plugin| plugin.tools.iter())
        .filter(|tool| tool.gateway)
        .count();

    assert_eq!(manifest.counts.tools, tool_count);
    assert_eq!(manifest.counts.gateway_tools, 5);
    assert_eq!(gateway_count, 5);
    assert_eq!(
        manifest.counts.execution_tools,
        manifest.counts.tools.saturating_sub(5)
    );
    for (plugin_id, canonical_name) in [
        ("agena.chatgpt", "agena.chatgpt.web_search"),
        ("agena.gemini", "agena.gemini.google_search"),
        ("agena.claude", "agena.claude.bash"),
    ] {
        assert!(manifest.plugins.iter().any(|plugin| {
            plugin.id == plugin_id
                && plugin
                    .tools
                    .iter()
                    .any(|tool| tool.canonical_name == canonical_name && !tool.gateway)
        }));
    }
    assert!(
        !manifest
            .plugins
            .iter()
            .any(|plugin| plugin.id == "agena.openai")
    );
}
