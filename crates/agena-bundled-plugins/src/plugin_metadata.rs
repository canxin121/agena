//! Presentation helpers shared by the bundled plugin declarations.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Embed a plugin's declared SVG in its manifest, so every client receives the
/// same artwork without a dependency on a Web-specific plugin-ID table.
pub(crate) fn icon(svg: &str) -> String {
    format!("data:image/svg+xml;base64,{}", STANDARD.encode(svg))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_bundled_plugin_declares_complete_presentation_metadata() {
        let manifests = crate::capability_manifest::bundled_plugin_manifests();
        let registered: BTreeSet<_> = crate::plugins::sources::bundled_plugin_entries()
            .into_keys()
            .collect();
        let declared: BTreeSet<_> = manifests
            .iter()
            .map(|(manifest, _)| format!("{}.{}", manifest.namespace, manifest.name))
            .collect();
        assert_eq!(
            declared, registered,
            "metadata coverage must include every registered plugin"
        );

        for (manifest, _) in manifests {
            let id = format!("{}.{}", manifest.namespace, manifest.name);
            assert!(
                manifest
                    .display_name
                    .as_deref()
                    .is_some_and(|name| !name.trim().is_empty()),
                "{id}: display name"
            );
            assert!(!manifest.version.trim().is_empty(), "{id}: version");
            assert_eq!(manifest.authors, ["canxin121"], "{id}: authors");
            assert!(
                manifest
                    .summary
                    .as_deref()
                    .is_some_and(|summary| !summary.trim().is_empty()),
                "{id}: summary"
            );
            assert!(
                manifest
                    .help
                    .as_deref()
                    .is_some_and(|help| !help.trim().is_empty()),
                "{id}: help"
            );
            assert!(!manifest.tags.is_empty(), "{id}: tags");
            let chinese = manifest
                .translations
                .get("zh-CN")
                .expect("Chinese plugin documentation");
            assert!(
                chinese
                    .summary
                    .as_deref()
                    .is_some_and(|summary| !summary.trim().is_empty()),
                "{id}: Chinese summary"
            );
            assert!(
                chinese
                    .help
                    .as_deref()
                    .is_some_and(|help| !help.trim().is_empty()),
                "{id}: Chinese help"
            );

            let icon = manifest.icon.as_deref().expect("bundled plugin icon");
            let encoded = icon
                .strip_prefix("data:image/svg+xml;base64,")
                .expect("self-contained SVG data URI");
            let svg = String::from_utf8(STANDARD.decode(encoded).expect("valid base64 icon"))
                .expect("UTF-8 SVG");
            assert!(
                svg.starts_with("<svg ") && svg.trim_end().ends_with("</svg>"),
                "{id}: SVG document"
            );
            assert!(svg.contains("fill=\"#"), "{id}: explicit image color");
            assert!(
                !svg.contains("currentColor"),
                "{id}: image color must not depend on host CSS"
            );
            assert_eq!(
                agena_plugin_host::sdk::normalize_plugin_icon(icon).as_deref(),
                Some(icon),
                "{id}: supported icon source"
            );
        }
    }
}
