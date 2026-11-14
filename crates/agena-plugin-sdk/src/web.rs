//! Native browser pages contributed by an installed, trusted plugin.
//!
//! Entries are ES modules exporting `mount(container, sdk)`. The host owns
//! resource loading, page identity, authentication, and instance lifetime.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{CommandDefinition, CommandTarget};

/// Version of the browser module and SDK contract implemented by this host.
pub const PLUGIN_WEB_API_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginWebContributions {
    pub api_version: u32,
    pub assets: PluginWebAssets,
    pub pages: Vec<PluginWebPage>,
}

impl Default for PluginWebContributions {
    fn default() -> Self {
        Self {
            api_version: PLUGIN_WEB_API_VERSION,
            assets: PluginWebAssets::default(),
            pages: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginWebAssets {
    /// Relative directory inside the host-configured package root.
    pub root: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginWebPage {
    /// Stable URL segment unique within this plugin.
    pub id: String,
    pub title: String,
    /// ES module path relative to the declared assets root.
    pub entry: String,
    #[serde(default)]
    pub styles: Vec<String>,
    #[serde(default)]
    pub placement: PluginWebPlacement,
    #[serde(default)]
    pub scope: PluginWebScope,
    /// Method commands this page may invoke through the host SDK.
    #[serde(default)]
    pub commands: Vec<String>,
    /// Whether the page may read and update the plugin's settings.
    #[serde(default)]
    pub settings: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginWebPlacement {
    #[default]
    Workspace,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginWebScope {
    /// Plugin-owned data; no conversation or project is implicitly selected.
    #[default]
    Instance,
    /// Requires an explicitly selected session belonging to a project.
    Session,
}

/// A portable URL-relative asset name. Percent escapes, empty components,
/// absolute paths, and dot components are rejected before filesystem access.
pub fn valid_web_asset_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 2048
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/' | b'@')
        })
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

impl PluginWebContributions {
    /// Validate declarations before loading any browser code or activating
    /// the plugin. Dynamic commands cannot silently widen a page's API.
    pub fn validate(
        &self,
        commands: &[CommandDefinition],
        has_settings: bool,
    ) -> Result<(), String> {
        if self.api_version != PLUGIN_WEB_API_VERSION {
            return Err(format!(
                "unsupported Web API version {}; expected {}",
                self.api_version, PLUGIN_WEB_API_VERSION
            ));
        }
        if !valid_web_asset_path(&self.assets.root) {
            return Err("Web assets.root must be a safe package-relative path".into());
        }
        if self.pages.is_empty() || self.pages.len() > 32 {
            return Err("Web contributions require between 1 and 32 pages".into());
        }
        let mut ids = BTreeSet::new();
        for page in &self.pages {
            if page.id.is_empty()
                || page.id.len() > 80
                || !page
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                || !ids.insert(&page.id)
            {
                return Err(format!("invalid or duplicate Web page id `{}`", page.id));
            }
            if page.title.trim().is_empty()
                || page.title != page.title.trim()
                || page.title.chars().count() > 200
            {
                return Err(format!(
                    "Web page `{}` needs a title of 1–200 characters",
                    page.id
                ));
            }
            if !valid_web_asset_path(&page.entry)
                || !(page.entry.ends_with(".js") || page.entry.ends_with(".mjs"))
            {
                return Err(format!(
                    "Web page `{}` needs a relative JavaScript entry",
                    page.id
                ));
            }
            let mut styles = BTreeSet::new();
            for style in &page.styles {
                if !valid_web_asset_path(style) || !style.ends_with(".css") || !styles.insert(style)
                {
                    return Err(format!("invalid or duplicate Web stylesheet `{style}`"));
                }
            }
            let mut exposed = BTreeSet::new();
            for command_id in &page.commands {
                let command = commands
                    .iter()
                    .find(|command| command.id == *command_id)
                    .ok_or_else(|| {
                        format!(
                            "Web page `{}` refers to undeclared command `{command_id}`",
                            page.id
                        )
                    })?;
                if !exposed.insert(command_id)
                    || !matches!(command.target, CommandTarget::Method { .. })
                {
                    return Err(format!(
                        "Web page `{}` requires distinct Method commands: `{command_id}`",
                        page.id
                    ));
                }
            }
            if page.settings && !has_settings {
                return Err(format!(
                    "Web page `{}` exposes settings but the plugin declares none",
                    page.id
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration() -> PluginWebContributions {
        PluginWebContributions {
            assets: PluginWebAssets {
                root: "web/dist".into(),
            },
            pages: vec![PluginWebPage {
                id: "dashboard".into(),
                title: "Dashboard".into(),
                entry: "assets/dashboard.js".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn legacy_surface_and_minimal_native_page_remain_distinct() {
        assert!(declaration().validate(&[], false).is_ok());
        let page: PluginWebPage = serde_json::from_value(serde_json::json!({
            "id": "dashboard", "title": "Dashboard", "entry": "index.mjs"
        }))
        .unwrap();
        assert_eq!(page.scope, PluginWebScope::Instance);
        assert!(page.commands.is_empty());
        assert!(!page.settings);
    }

    #[test]
    fn web_paths_reject_url_and_filesystem_escapes() {
        for path in [
            "",
            "/",
            "/app.js",
            "../app.js",
            "a/../app.js",
            "./app.js",
            "a//b.js",
            "a\\b.js",
            "a/%2e%2e/b.js",
            "https://host/a.js",
            "app.js?token=x",
            "app.js#fragment",
            "C:/app.js",
        ] {
            assert!(!valid_web_asset_path(path), "{path}");
        }
        assert!(valid_web_asset_path("chunks/main-AB_12.js"));
    }

    #[test]
    fn pages_expose_only_distinct_declared_method_commands() {
        let mut command: CommandDefinition = serde_json::from_value(serde_json::json!({
            "id": "read", "title": "Read", "group": "Web",
            "input": crate::SettingsContract::empty_object("Read", "Read data"),
            "target": {"kind": "method", "handler": "read"}
        }))
        .unwrap();
        let mut web = declaration();
        web.pages[0].commands = vec!["read".into()];
        assert!(web.validate(&[command.clone()], false).is_ok());
        web.pages[0].commands.push("read".into());
        assert!(web.validate(&[command.clone()], false).is_err());
        web.pages[0].commands.pop();
        command.target = CommandTarget::Tool {
            tool: "read".into(),
        };
        assert!(web.validate(&[command], false).is_err());
    }

    #[test]
    fn activation_rejects_unsupported_versions_duplicates_and_missing_commands() {
        let mut web = declaration();
        web.api_version = 2;
        assert!(web.validate(&[], false).unwrap_err().contains("version"));
        web.api_version = 1;
        web.pages.push(web.pages[0].clone());
        assert!(web.validate(&[], false).unwrap_err().contains("duplicate"));
        web.pages.pop();
        web.pages[0].commands.push("missing".into());
        assert!(web.validate(&[], false).unwrap_err().contains("undeclared"));
        web.pages[0].commands.clear();
        web.pages[0].settings = true;
        assert!(web.validate(&[], false).unwrap_err().contains("settings"));
    }
}
