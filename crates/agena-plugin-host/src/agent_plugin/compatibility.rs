use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::AgentPluginDescriptor;

#[derive(Debug, Clone, Serialize)]
pub struct AgentPluginCompatibility {
    pub name: String,
    pub format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub components: BTreeMap<&'static str, usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cordis_dependencies: Vec<CordisDependency>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CordisDependency {
    pub entry: String,
    pub module: String,
    #[serde(flatten)]
    pub service: CordisService,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CordisService {
    pub name: String,
    pub status: CordisServiceStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CordisServiceStatus {
    Available,
    Unavailable,
    UnsupportedIntercept,
}

impl AgentPluginDescriptor {
    pub fn compatibility(&self) -> AgentPluginCompatibility {
        let output_styles = self.roles.iter().filter(|role| role.output_style).count();
        let components = [
            ("agents", self.roles.len() - output_styles),
            ("bin directories", usize::from(self.bin_dir.is_some())),
            ("commands", self.commands.len()),
            ("Cordis skills", self.cordis_skill_count),
            (
                "Cordis tools",
                self.deepseek_programs
                    .iter()
                    .map(|program| program.tools().len())
                    .sum(),
            ),
            ("hooks", self.hooks.len()),
            ("language servers", self.lsp_servers.len()),
            (
                "managed MCP entries",
                self.deepseek_managed_mcp_servers.len(),
            ),
            ("MCP servers", self.mcp_servers.len()),
            ("MemVault integrations", self.memvault.len()),
            ("mod bundles", usize::from(self.mods.is_some())),
            ("monitors", self.monitors.len()),
            ("output styles", output_styles),
            ("skill directories", self.skill_dirs.len()),
            ("terminal themes", self.themes.len()),
            ("workflows", self.workflows.len()),
        ]
        .into_iter()
        .filter(|(_, count)| *count != 0)
        .collect();
        AgentPluginCompatibility {
            name: self.name.clone(),
            format: self.format.as_str(),
            version: self.version.clone(),
            components,
            cordis_dependencies: self.cordis_dependencies.clone(),
        }
    }
}

impl AgentPluginCompatibility {
    pub fn component_summary(&self) -> String {
        if self.components.is_empty() {
            return "No plugin components discovered.".to_owned();
        }
        let components = self
            .components
            .iter()
            .map(|(label, count)| format!("{label} ({count})"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("Discovered components: {components}.")
    }
}
