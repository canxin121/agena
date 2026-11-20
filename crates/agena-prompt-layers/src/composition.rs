use super::*;

const MAX_DEPTH: usize = 32;
pub(super) const MAX_COMPONENTS: usize = 256;

impl PromptFragment {
    pub(super) fn composition_error(&self, reason: impl Into<String>) -> PromptError {
        PromptError::InvalidComposition {
            id: self.id.to_string(),
            reason: reason.into(),
        }
    }
}

/// Validate saved references even when a composition is not active in a layer.
pub fn validate_compositions(
    library: &BTreeMap<FragmentId, PromptFragment>,
    scope: PromptScope,
) -> Result<(), PromptError> {
    let layer = PromptLayer::default();
    let source = LayerSource::new(scope, &layer, library);
    for fragment in library
        .values()
        .filter(|fragment| !fragment.parts.is_empty())
    {
        let mut bytes = 0;
        walk(
            fragment,
            &source,
            &mut BTreeSet::new(),
            &mut 0,
            &mut |part| {
                let body = part.body.trim();
                if part.parts.is_empty() && !body.is_empty() {
                    bytes += body.len() + if bytes == 0 { 0 } else { 2 };
                    if bytes > MAX_TOTAL_BYTES {
                        return Err(fragment.composition_error(format!(
                            "combined bodies exceed {MAX_TOTAL_BYTES} bytes"
                        )));
                    }
                }
                Ok(true)
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> FragmentId {
        FragmentId::parse(name).unwrap()
    }

    fn composed(name: &str, parts: &[&str]) -> PromptFragment {
        let mut fragment = PromptFragment::new(id(name), "");
        fragment.parts = parts.iter().map(|name| id(name)).collect();
        fragment
    }

    #[test]
    fn nested_references_render_in_order_and_keep_conditions() {
        let mut with_tools = PromptFragment::new(id("tools"), "Cite the retrieved sources.");
        with_tools.when.requires_tools = Some(true);
        let mut library: BTreeMap<_, _> = [
            PromptFragment::new(id("opening"), "Start here."),
            with_tools,
            PromptFragment::new(id("close"), "End here."),
            composed("inner", &["opening", "tools"]),
            composed("answer", &["inner", "close"]),
        ]
        .into_iter()
        .map(|fragment| (fragment.id.clone(), fragment))
        .collect();
        let layer = PromptLayer {
            order: vec![id("answer")],
            ..Default::default()
        };
        let resolve_answer = |library: &_, tools_enabled| {
            resolve(
                &[LayerSource::new(PromptScope::Global, &layer, library)],
                &PromptContext {
                    tools_enabled,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        validate_compositions(&library, PromptScope::Global).unwrap();
        let offline = resolve_answer(&library, false);
        assert_eq!(offline.text, "Start here.\n\nEnd here.");
        assert_eq!(offline.warnings[0].id, id("tools"));
        library.get_mut(&id("close")).unwrap().body = "Updated ending.".into();
        assert_eq!(
            resolve_answer(&library, true).text,
            "Start here.\n\nCite the retrieved sources.\n\nUpdated ending."
        );
    }

    #[test]
    fn unused_compositions_are_checked_for_missing_parts_and_cycles() {
        let mut library = BTreeMap::from([(id("a"), composed("a", &["b"]))]);
        let error = validate_compositions(&library, PromptScope::Global).unwrap_err();
        assert!(error.to_string().contains("component `b` is missing"));
        library.insert(id("b"), composed("b", &["a"]));
        let error = validate_compositions(&library, PromptScope::Global).unwrap_err();
        assert!(error.to_string().contains("form a cycle"));
    }
}

fn walk<'a>(
    fragment: &'a PromptFragment,
    source: &'a LayerSource<'_>,
    ancestors: &mut BTreeSet<&'a FragmentId>,
    count: &mut usize,
    visit: &mut impl FnMut(&PromptFragment) -> Result<bool, PromptError>,
) -> Result<(), PromptError> {
    if ancestors.contains(&fragment.id) {
        return Err(fragment.composition_error("prompt references form a cycle"));
    }
    *count += 1;
    if ancestors.len() >= MAX_DEPTH || *count > MAX_COMPONENTS {
        return Err(fragment.composition_error(format!(
            "limit of {MAX_COMPONENTS} components and {MAX_DEPTH} nesting levels exceeded"
        )));
    }
    fragment.validate()?;
    if !visit(fragment)? || fragment.parts.is_empty() {
        return Ok(());
    }
    ancestors.insert(&fragment.id);
    for id in &fragment.parts {
        let part = source
            .layer
            .inline_fragment(id)
            .or_else(|| source.library.get(id))
            .ok_or_else(|| fragment.composition_error(format!("component `{id}` is missing")))?;
        walk(part, source, ancestors, count, visit)?;
    }
    ancestors.remove(&fragment.id);
    Ok(())
}

pub(super) fn render_fragment(
    root: &PromptFragment,
    source: &LayerSource<'_>,
    context: &PromptContext,
    warnings: &mut Vec<PromptWarning>,
) -> Result<Option<String>, PromptError> {
    let empty = empty_route();
    let route = context.route.as_ref().unwrap_or(&empty);
    let mut text = None::<String>;
    walk(
        root,
        source,
        &mut BTreeSet::new(),
        &mut 0,
        &mut |fragment| {
            if !fragment.when.matches(route, context.tools_enabled) {
                warnings.push(PromptWarning {
                    scope: source.scope,
                    id: fragment.id.clone(),
                    kind: PromptWarningKind::ConditionNotMatched,
                    detail: if fragment.when.models.is_empty() {
                        "requires_tools".to_owned()
                    } else {
                        fragment.when.models.join(", ")
                    },
                });
                return Ok(false);
            }
            if fragment.parts.is_empty() {
                let (rendered, unknown) = render(&fragment.body, context);
                if !unknown.is_empty() {
                    warnings.push(PromptWarning {
                        scope: source.scope,
                        id: fragment.id.clone(),
                        kind: PromptWarningKind::UnknownVariable,
                        detail: unknown.join(", "),
                    });
                }
                let output = text.get_or_insert_with(String::new);
                if root.parts.is_empty() {
                    *output = rendered;
                } else if !rendered.trim().is_empty() {
                    if !output.is_empty() {
                        output.push_str("\n\n");
                    }
                    output.push_str(rendered.trim());
                    if output.len() > MAX_TOTAL_BYTES {
                        return Err(PromptError::TotalTooLarge {
                            bytes: output.len(),
                        });
                    }
                }
            }
            Ok(true)
        },
    )?;
    Ok(text)
}
