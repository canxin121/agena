use crossterm::event::KeyCode;
use serde_json::Value;

use crate::{App, BTreeMap, ComposerItem, KeyEvent, Route, SkillPickerOverlay, UiResult, ui_text};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::selection_picker::SelectionPickerItem;

const SKILL_PICKER_PAGE_SIZE: usize = 12;

#[derive(Debug)]
struct SkillCatalogItem {
    name: String,
    summary: String,
    aliases: Vec<String>,
}

impl App {
    /// Opens the user-driven Skill attachment flow. A Skill is read only after
    /// the user selects it, and its exact text is then staged on the composer
    /// as a message-scoped snapshot.
    pub(crate) fn open_skill_picker(&mut self) {
        let Some(session_id) = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
        else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };

        let presentation = agena_tui::selection_picker::new_presentation(
            ui_text::t(&self.i18n, "overlay-skill-picker-title"),
            ui_text::t(&self.i18n, "overlay-skill-picker-prompt"),
            String::new(),
            ui_text::t(&self.i18n, "overlay-skill-picker-empty"),
            String::new(),
        );
        let mut dialog = SkillPickerOverlay {
            presentation,
            session_id,
            actions: BTreeMap::new(),
            offset: 0,
            total: 0,
            limit: SKILL_PICKER_PAGE_SIZE,
        };
        self.load_skill_picker_page(&mut dialog, 0);
        self.current_route = Route::SkillPicker(dialog);
    }

    pub(crate) fn handle_skill_picker_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SkillPickerOverlay,
    ) -> bool {
        match key.code {
            KeyCode::PageUp if dialog.offset > 0 => {
                self.load_skill_picker_page(dialog, dialog.offset.saturating_sub(dialog.limit));
                return false;
            }
            KeyCode::PageDown
                if dialog
                    .offset
                    .saturating_add(dialog.presentation.items.len())
                    < dialog.total =>
            {
                self.load_skill_picker_page(dialog, dialog.offset.saturating_add(dialog.limit));
                return false;
            }
            _ => {}
        }

        let action = match resolve_tui_key(KeyContext::Picker, key) {
            Some(KeyAction::Accept) => agena_tui::selection_picker::SelectionPickerAction::Accept,
            _ => agena_tui::selection_picker::SelectionPickerAction::Input(key),
        };
        match agena_tui::selection_picker::reduce(&mut dialog.presentation, action) {
            agena_tui::selection_picker::SelectionPickerEffect::Close => true,
            agena_tui::selection_picker::SelectionPickerEffect::KeepOpen => false,
            agena_tui::selection_picker::SelectionPickerEffect::Activate { key } => {
                let Some(name) = dialog.actions.get(key.as_str()).cloned() else {
                    return false;
                };
                match self.load_skill_snapshot(dialog.session_id, name.as_str()) {
                    Ok(skill) => {
                        self.stage_skill_reference(skill);
                        self.focus = agena_tui::main_focus::Focus::Composer;
                        true
                    }
                    Err(error) => {
                        dialog.presentation.error_message = Some(error);
                        false
                    }
                }
            }
        }
    }

    fn load_skill_picker_page(&mut self, dialog: &mut SkillPickerOverlay, offset: usize) {
        dialog.presentation.set_loading(true);
        dialog.presentation.error_message = None;
        dialog.actions.clear();

        let result = self.block_on_async(self.backend.invoke_plugin_ui_tool(
            "agena.skills",
            "list",
            serde_json::json!({
                "kind": "skill",
                "offset": offset,
                "limit": dialog.limit,
                "verbose": false,
            }),
            Some(dialog.session_id),
        ));
        match result.and_then(|response| skill_catalog_page(response.payload)) {
            Ok(page) => {
                dialog.offset = page.offset;
                dialog.total = page.total;
                let mut items = Vec::with_capacity(page.items.len());
                for (index, skill) in page.items.into_iter().enumerate() {
                    let key = format!("skill:{}:{}", dialog.offset, index);
                    let detail = if skill.aliases.is_empty() {
                        skill.summary.clone()
                    } else if skill.summary.is_empty() {
                        format!("aliases: {}", skill.aliases.join(", "))
                    } else {
                        format!("{} · aliases: {}", skill.summary, skill.aliases.join(", "))
                    };
                    items.push(SelectionPickerItem::new(
                        key.clone(),
                        skill.name.clone(),
                        detail.clone(),
                        format!("{} {} {}", skill.name, detail, skill.aliases.join(" ")),
                    ));
                    dialog.actions.insert(key, skill.name);
                }
                dialog.presentation.replace_items(items);
                dialog.presentation.footer = self.skill_picker_footer(dialog);
                dialog.presentation.set_loading(false);
            }
            Err(error) => {
                dialog.presentation.replace_items(Vec::new());
                dialog.presentation.error_message = Some(error);
                dialog.presentation.footer = self.skill_picker_footer(dialog);
                dialog.presentation.set_loading(false);
            }
        }
    }

    fn load_skill_snapshot(&mut self, session_id: i64, name: &str) -> UiResult<ComposerItem> {
        let response = self
            .block_on_async(self.backend.invoke_plugin_ui_tool(
                "agena.skills",
                "get",
                serde_json::json!({ "name": name }),
                Some(session_id),
            ))
            .map_err(|error| error.to_string())?;
        skill_snapshot(response.payload)
    }

    fn skill_picker_footer(&self, dialog: &SkillPickerOverlay) -> String {
        let page = dialog.offset / dialog.limit + 1;
        let pages = dialog.total.max(1).div_ceil(dialog.limit);
        self.i18n.text_args(
            "overlay-skill-picker-footer",
            &agena_tui::fl_args!("page" => page as i64, "pages" => pages as i64, "total" => dialog.total as i64),
        )
    }
}

fn skill_catalog_page(payload: Option<Value>) -> UiResult<SkillCatalogPage> {
    let payload = payload
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| "Skill catalog returned no structured payload.".to_owned())?;
    let total = json_count(payload.get("total"));
    let offset = json_count(payload.get("offset"));
    let tools = payload
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| "Skill catalog payload is missing its tools list.".to_owned())?;
    let items = tools
        .iter()
        .filter_map(|value| {
            let value = value.as_object()?;
            (json_string(value.get("kind")) == "skill").then_some(())?;
            let name = json_string(value.get("name"));
            (!name.is_empty()).then(|| SkillCatalogItem {
                name,
                summary: json_string(value.get("summary")),
                aliases: json_strings(value.get("aliases")),
            })
        })
        .collect();
    Ok(SkillCatalogPage {
        items,
        total,
        offset,
    })
}

fn skill_snapshot(payload: Option<Value>) -> UiResult<ComposerItem> {
    let payload = payload
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| "Skill details returned no structured payload.".to_owned())?;
    if json_string(payload.get("kind")) != "skill" {
        return Err("The selected catalog entry is not a Skill.".to_owned());
    }
    let name = required_skill_field(payload, "name")?;
    let instructions = required_skill_field(payload, "body")?;
    let content_hash = required_skill_field(payload, "content_hash")?;
    let source = required_skill_field(payload, "source")?;
    Ok(ComposerItem {
        placeholder: format!("[Skill: {name}]"),
        label: format!("Skill: {name}"),
        activity: agena_domain::ComposerActivity {
            id: agena_domain::ActivityId::new(),
            payload: agena_domain::ActivityPayload::SkillReference(
                agena_domain::SkillReferenceActivity {
                    description: json_string(payload.get("summary")),
                    aliases: json_strings(payload.get("aliases")),
                    name: name.clone(),
                    instructions,
                    content_hash: content_hash.clone(),
                    source: source.clone(),
                },
            ),
            provenance: agena_domain::ActivityProvenance {
                source: Some(source),
                content_hash: Some(content_hash),
                plugin_id: Some("agena.skills".to_owned()),
            },
        },
    })
}

#[derive(Debug)]
struct SkillCatalogPage {
    items: Vec<SkillCatalogItem>,
    total: usize,
    offset: usize,
}

fn required_skill_field(payload: &serde_json::Map<String, Value>, field: &str) -> UiResult<String> {
    let value = json_string(payload.get(field));
    (!value.is_empty())
        .then_some(value)
        .ok_or_else(|| format!("Skill detail is missing required field `{field}`."))
}

fn json_string(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn json_strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn json_count(value: Option<&Value>) -> usize {
    value
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{skill_catalog_page, skill_snapshot};

    #[test]
    fn parses_the_skill_catalog_page_without_activation_metadata() {
        let page = skill_catalog_page(Some(serde_json::json!({
            "tools": [{
                "name": "review",
                "kind": "skill",
                "summary": "Review a change",
                "aliases": ["code-review"],
            }],
            "total": 1,
            "offset": 0,
        })))
        .expect("valid catalog page");
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "review");
        assert_eq!(page.items[0].aliases, ["code-review"]);
    }

    #[test]
    fn parses_an_immutable_skill_snapshot() {
        let skill = skill_snapshot(Some(serde_json::json!({
            "name": "review",
            "kind": "skill",
            "summary": "Review a change",
            "body": "Inspect the diff.",
            "aliases": ["code-review"],
            "content_hash": "abc123",
            "source": "workspace",
        })))
        .expect("valid skill snapshot");
        let agena_domain::ActivityPayload::SkillReference(reference) = &skill.activity.payload
        else {
            panic!("expected a Skill reference activity")
        };
        assert_eq!(reference.name, "review");
        assert_eq!(reference.instructions, "Inspect the diff.");
        assert_eq!(reference.content_hash, "abc123");
        assert_eq!(reference.source, "workspace");
        assert_eq!(
            skill.activity.provenance.source.as_deref(),
            Some("workspace")
        );
        assert_eq!(
            skill.activity.provenance.content_hash.as_deref(),
            Some("abc123")
        );
    }
}
