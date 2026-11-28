use std::collections::{BTreeMap, BTreeSet};

use agena_domain::{ActivityPayload, CommandReferenceActivity, ComposerDocument, ComposerNode};
use agena_plugin_host::AgentPluginSkill;
use agena_storage::kernel::{
    MAX_USER_CONTEXT_BLOCK_BYTES, MAX_USER_CONTEXT_BLOCKS, MAX_USER_CONTEXT_BYTES,
};

use super::{AppError, ToolExecutor};

const SKILLS_PLUGIN: &str = "agena.skills";

fn declared_reference(payload: &ActivityPayload) -> Option<&CommandReferenceActivity> {
    let ActivityPayload::CommandReference(reference) = payload else {
        return None;
    };
    reference
        .skill_id
        .as_deref()
        .is_some_and(|id| id.starts_with("declared-"))
        .then_some(reference)
}

fn references(document: &ComposerDocument) -> impl Iterator<Item = &CommandReferenceActivity> {
    document.0.iter().filter_map(|node| match node {
        ComposerNode::Activity { activity } => declared_reference(&activity.payload),
        _ => None,
    })
}

pub(super) fn has_declared_reference(document: &ComposerDocument) -> bool {
    references(document).next().is_some()
}

pub(super) fn typed_skill_names(document: &ComposerDocument, literal_leading_slash: bool) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();
    let mut leading = literal_leading_slash;
    for node in &document.0 {
        let ComposerNode::Text { text } = node else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let skip = std::mem::take(&mut leading) && text.starts_with('/');
        for word in text.split_whitespace().skip(usize::from(skip)) {
            if let Some(name) = word.strip_prefix('/')
                && name.len() <= 64
                && name.split('-').all(|segment| {
                    !segment.is_empty()
                        && segment.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                })
                && seen.insert(name)
            {
                names.push(name.to_owned());
            }
        }
    }
    names
}

/// Store the resolved skill bodies with the message so replay uses the same text.
pub(super) async fn selected_skill_context(
    executor: &ToolExecutor,
    document: &mut ComposerDocument,
    operator: bool,
    typed_names: &[String],
) -> Result<Vec<String>, AppError> {
    if !has_declared_reference(document) && typed_names.is_empty() {
        return Ok(Vec::new());
    }
    let unavailable = || {
        AppError::Config(
            "A selected plugin skill is unavailable in this session. Refresh the skill picker and select it again."
                .into(),
        )
    };
    if !operator || executor.is_global() {
        return if references(document).any(|reference| reference.source != SKILLS_PLUGIN) {
            Err(unavailable())
        } else {
            Ok(Vec::new())
        };
    }

    let provided = executor.agent_plugin_provided_skills().await;
    let skills: BTreeMap<_, _> = provided
        .iter()
        .map(|skill| (skill.id(), skill))
        .collect();
    let disabled = executor
        .plugin_manager()
        .plugins()
        .iter()
        .find(|plugin| plugin.key().to_string() == "agena.commands")
        .and_then(|plugin| plugin.configured_plugin.settings().get("disabled"))
        .and_then(|value| serde_json::from_value::<Vec<String>>(value.clone()).ok())
        .unwrap_or_default();
    let disabled_by_name = |name: &str| {
        disabled.iter().any(|disabled| disabled.trim().eq_ignore_ascii_case(name))
    };
    let user_invocable = |skill: &AgentPluginSkill| {
        skill.invocation.is_some_and(|policy| policy.user_invocable)
            && !disabled_by_name(&skill.name)
    };
    let mut used = BTreeSet::new();
    let mut selected_names = BTreeSet::new();
    let mut context = Vec::new();
    let mut remaining = MAX_USER_CONTEXT_BYTES;
    for node in &mut document.0 {
        let ComposerNode::Activity { activity } = node else {
            continue;
        };
        let ActivityPayload::CommandReference(reference) = &mut activity.payload else {
            continue;
        };
        let Some(id) = &reference.skill_id else {
            continue;
        };
        if !id.starts_with("declared-") {
            continue;
        }
        let Some(skill) = skills.get(id) else {
            // Older clients also use this source for ordinary manifest
            // declarations, which keep their lazy lookup.
            if reference.source == SKILLS_PLUGIN {
                continue;
            }
            return Err(unavailable());
        };
        if (reference.source != SKILLS_PLUGIN && reference.source != skill.plugin_id)
            || reference.name != skill.name
            || !user_invocable(skill)
        {
            return Err(unavailable());
        }
        if reference.content_hash != skill.summary().revision {
            return Err(AppError::Config(format!(
                "The selected skill '{}' changed. Refresh the skill picker and select it again.",
                skill.name
            )));
        }
        reference.source.clone_from(&skill.plugin_id);
        activity.provenance.source = Some(skill.plugin_id.clone());
        activity.provenance.plugin_id = Some(skill.plugin_id.clone());
        activity.provenance.content_hash = Some(reference.content_hash.clone());
        if !used.insert(id) {
            continue;
        }
        selected_names.insert(skill.name.as_str());
        append_skill(skill, &mut context, &mut remaining)?;
    }
    if !typed_names.is_empty() {
        let files = executor.deepseek_file_skills(typed_names).await;
        // A Cordis registration takes precedence over a bundled file of the
        // same name. Select its definition before checking invocation policy.
        let by_name: BTreeMap<_, _> = files.iter().chain(&provided)
            .map(|skill| (skill.name.as_str(), skill))
            .collect();
        for name in typed_names {
            if !selected_names.contains(name.as_str())
                && let Some(skill) = by_name.get(name.as_str())
                && user_invocable(skill)
            {
                append_skill(skill, &mut context, &mut remaining)?;
            }
        }
    }
    Ok(context)
}

fn append_skill(
    skill: &AgentPluginSkill,
    context: &mut Vec<String>,
    remaining: &mut usize,
) -> Result<(), AppError> {
    let mut block = format!(
        "<agena_selected_skill>\nSkill {} from {}\n",
        skill.name, skill.plugin_id
    );
    if let Some(base) = &skill.resource_base {
        block.push_str(&base.hint());
        block.push('\n');
    }
    block.push('\n');
    block.push_str(&skill.instructions);
    block.push_str("\n</agena_selected_skill>");
    if block.len() > MAX_USER_CONTEXT_BLOCK_BYTES
        || block.len() > *remaining
        || context.len() >= MAX_USER_CONTEXT_BLOCKS
    {
        return Err(AppError::Config(
            "The selected skills exceed the message instruction budget.".into(),
        ));
    }
    *remaining -= block.len();
    context.push(block);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> ComposerDocument {
        ComposerDocument(vec![ComposerNode::Text {
            text: text.to_owned(),
        }])
    }

    #[test]
    fn typed_skill_names_parses_kebab_tokens_and_respects_leading_escape() {
        assert_eq!(
            typed_skill_names(
                &doc("/first check /second-2 /first /usr/bin https://example.test/skip //escaped /Upper /-bad /bad- /two--parts /"),
                false,
            ),
            vec!["first", "second-2"]
        );
        assert_eq!(
            typed_skill_names(&doc("/first and /second"), true),
            vec!["second"]
        );
        assert_eq!(
            typed_skill_names(&doc("plain /first"), true),
            vec!["first"]
        );
        let long = format!("/{}", "a".repeat(65));
        assert!(typed_skill_names(&doc(&long), false).is_empty());
    }
}
