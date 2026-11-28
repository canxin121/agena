use std::{collections::BTreeMap, sync::Arc};

use agena_plugin_sdk::{HostClient, PluginError, Result};
use serde::Deserialize;

use super::Program;

pub(super) const MODULE: &str = include_str!("prompt.js");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::agent_plugin) struct AssembledPrompt {
    pub text: String,
    pub complete: bool,
}

#[derive(Deserialize)]
struct Assembly {
    sections: Vec<Section>,
    variables: Vec<(String, Option<String>)>,
}

#[derive(Deserialize)]
struct Section {
    name: String,
    order: f64,
    text: String,
    interpolate: bool,
    #[serde(default)]
    complete: bool,
}

pub(in crate::agent_plugin) async fn assemble(
    programs: &[Arc<Program>],
    session: i64,
    host: &dyn HostClient,
    max_bytes: usize,
) -> Result<AssembledPrompt> {
    let mut sections = BTreeMap::new();
    let mut variables = BTreeMap::new();
    for program in programs
        .iter()
        .filter(|program| program.uses_system_prompt())
    {
        let assembly: Assembly = program
            .invoke(
                "assemblePrompt",
                Some(session),
                Some(host),
                b"null".to_vec(),
                None,
                None,
            )
            .await?;
        for (name, value) in assembly.variables {
            if variables.insert(name.clone(), value).is_some() {
                return Err(PluginError::invalid_params(format!(
                    "duplicate Cordis prompt variable {name:?}"
                )));
            }
        }
        for section in assembly.sections {
            let name = section.name.clone();
            if sections.insert(name.clone(), section).is_some() {
                return Err(PluginError::invalid_params(format!(
                    "duplicate Cordis prompt section {name:?}"
                )));
            }
        }
    }
    assemble_parts(sections.into_values().collect(), &variables, max_bytes)
}

fn assemble_parts(
    mut sections: Vec<Section>,
    variables: &BTreeMap<String, Option<String>>,
    max_bytes: usize,
) -> Result<AssembledPrompt> {
    sections.sort_by(|left, right| {
        left.order
            .total_cmp(&right.order)
            .then_with(|| left.name.encode_utf16().cmp(right.name.encode_utf16()))
    });
    let mut complete_sections = Vec::new();
    for section in sections.iter().filter(|section| section.complete) {
        let text = if section.interpolate {
            interpolate(section, variables, max_bytes)?
        } else {
            section.text.clone()
        };
        if !text.is_empty() {
            complete_sections.push((section.name.clone(), text));
        }
    }
    if complete_sections.len() > 1 {
        let names = complete_sections
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(PluginError::invalid_params(format!(
            "multiple complete prompt sections are active: {names}"
        )));
    }
    if let Some((_, text)) = complete_sections.into_iter().next() {
        if text.len() > max_bytes {
            return Err(PluginError::invalid_params(
                "Cordis system prompt exceeds its byte limit",
            ));
        }
        return Ok(AssembledPrompt {
            text,
            complete: true,
        });
    }
    let mut rendered = String::new();
    for section in sections.into_iter().filter(|section| !section.complete) {
        let text = if section.interpolate {
            interpolate(&section, variables, max_bytes)?
        } else {
            section.text
        };
        if !text.is_empty() {
            if !rendered.is_empty() {
                rendered.push_str("\n\n");
            }
            append(&mut rendered, &text, max_bytes)?;
        }
    }
    Ok(AssembledPrompt {
        text: rendered,
        complete: false,
    })
}

fn interpolate(
    section: &Section,
    variables: &BTreeMap<String, Option<String>>,
    max_bytes: usize,
) -> Result<String> {
    let mut rendered = String::new();
    let mut remaining = section.text.as_str();
    while let Some((prefix, group)) = remaining.split_once("{{") {
        let Some((name, tail)) = group.split_once("}}") else {
            break;
        };
        let value = variables
            .get(name)
            .and_then(Option::as_deref)
            .ok_or_else(|| {
                PluginError::invalid_params(format!(
                    "unresolved Cordis prompt variable {name:?} in section {:?}",
                    section.name
                ))
            })?;
        append(&mut rendered, prefix, max_bytes)?;
        append(&mut rendered, value, max_bytes)?;
        remaining = tail;
    }
    append(&mut rendered, remaining, max_bytes)?;
    Ok(rendered)
}

fn append(output: &mut String, text: &str, max_bytes: usize) -> Result<()> {
    if output.len().saturating_add(text.len()) > max_bytes {
        return Err(PluginError::invalid_params(
            "Cordis system prompt exceeds its byte limit",
        ));
    }
    output.push_str(text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(name: &str, order: f64, text: &str, interpolate: bool, complete: bool) -> Section {
        Section {
            name: name.into(),
            order,
            text: text.into(),
            interpolate,
            complete,
        }
    }

    #[test]
    fn complete_section_replaces_normal_sections_and_interpolates() {
        let vars = BTreeMap::from([("mode".into(), Some("strict".into()))]);
        let assembled = assemble_parts(
            vec![
                section("normal", 10.0, "ignored normal", true, false),
                section("full", 20.0, "Complete prompt ({{mode}})", true, true),
                section("inactive", 30.0, "", true, true),
            ],
            &vars,
            1024,
        )
        .unwrap();
        assert_eq!(
            assembled,
            AssembledPrompt {
                text: "Complete prompt (strict)".into(),
                complete: true,
            }
        );
    }

    #[test]
    fn inactive_complete_section_falls_back_to_normal_sections() {
        let assembled = assemble_parts(
            vec![
                section("normal", 10.0, "Normal body", false, false),
                section("inactive", 20.0, "", true, true),
            ],
            &BTreeMap::new(),
            1024,
        )
        .unwrap();
        assert_eq!(
            assembled,
            AssembledPrompt {
                text: "Normal body".into(),
                complete: false,
            }
        );
    }

    #[test]
    fn multiple_active_complete_sections_fail() {
        let error = assemble_parts(
            vec![
                section("second", 20.0, "B", false, true),
                section("first", 10.0, "A", false, true),
            ],
            &BTreeMap::new(),
            1024,
        )
        .unwrap_err();
        assert!(
            error
                .diagnostic_message()
                .contains("multiple complete prompt sections are active: first, second"),
            "{error}"
        );
    }
}
