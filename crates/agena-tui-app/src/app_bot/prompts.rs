use std::{cell::Cell, collections::BTreeMap};

use agena_prompt_layers::{BOT_INSTRUCTIONS_ID, BuiltinPrompt, PromptFragment, PromptLayer};
use agena_tui_components::theme;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::*;
use crate::app_prompt_studio::{
    move_list_selection, read_prompt_draft, split_prompt_catalog,
    view::{PromptRow, prompt_excerpt, render_detail, render_rows},
};

#[derive(Default)]
pub(crate) struct BotPromptCatalog {
    builtins: Vec<BuiltinPrompt>,
    global: PromptLayer,
    library: BTreeMap<String, PromptFragment>,
}

impl BotPromptCatalog {
    pub(super) fn new(
        builtins: Vec<BuiltinPrompt>,
        setting: &serde_json::Value,
    ) -> anyhow::Result<Self> {
        let (global, _) = read_prompt_draft(setting)?;
        let (builtins, reusable) = split_prompt_catalog(builtins);
        let library = global
            .library
            .into_iter()
            .chain(global.layer.inline.iter().cloned())
            .chain(reusable)
            .map(|prompt| (prompt.id.to_string(), prompt))
            .collect();
        Ok(Self {
            builtins,
            global: global.layer,
            library,
        })
    }
}

#[derive(Default)]
pub(super) struct BotPromptEditor {
    pub catalog: Option<BotPromptCatalog>,
    pub selected: usize,
    pub library: bool,
    pub detail: bool,
    scroll: Cell<u16>,
}

impl BotForm {
    fn selected_prompts(&self) -> &[String] {
        self.persona
            .prompts
            .as_ref()
            .map_or(&[], |selection| selection.ids.as_slice())
    }

    fn ordered_choices(&self) -> Vec<String> {
        let mut ids = self.selected_prompts().to_vec();
        if self.persona.instructions.trim().is_empty() {
            ids.retain(|id| id != BOT_INSTRUCTIONS_ID);
        } else if !ids.iter().any(|id| id == BOT_INSTRUCTIONS_ID) {
            ids.push(BOT_INSTRUCTIONS_ID.to_owned());
        }
        ids
    }

    fn prompt_order(&self) -> Vec<String> {
        let catalog = self.prompts.catalog.as_ref();
        let builtins = catalog.map_or(&[][..], |catalog| catalog.builtins.as_slice());
        let mut ids: Vec<_> = builtins
            .iter()
            .map(|prompt| prompt.id.to_string())
            .collect();
        let inherited = catalog
            .into_iter()
            .flat_map(|catalog| &catalog.global.order)
            .filter(|id| id.is_builtin())
            .map(|id| id.as_str());
        let choices = self.ordered_choices();
        for id in inherited.chain(choices.iter().map(String::as_str)) {
            let id = builtins
                .iter()
                .find(|prompt| prompt.matches_id(id))
                .map_or(id, |prompt| prompt.id.as_str());
            ids.retain(|entry| entry != id);
            ids.push(id.to_owned());
        }
        ids
    }

    fn prompt_library(&self) -> Vec<&PromptFragment> {
        let Some(catalog) = &self.prompts.catalog else {
            return Vec::new();
        };
        let mut library: Vec<_> = catalog
            .library
            .values()
            .filter(|fragment| {
                fragment.id.as_str() != BOT_INSTRUCTIONS_ID
                    && !self
                        .selected_prompts()
                        .iter()
                        .any(|id| id == fragment.id.as_str())
                    && !catalog.global.disabled.contains(&fragment.id)
            })
            .collect();
        library.sort_by(|left, right| {
            left.title
                .to_lowercase()
                .cmp(&right.title.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        library
    }

    fn set_prompt_order(&mut self, ids: Vec<String>) {
        self.persona
            .prompts
            .get_or_insert_with(BotPromptSelection::default)
            .ids = ids;
        self.request_key = Uuid::new_v4().to_string();
    }

    pub(super) fn has_shared_order(&self) -> bool {
        self.selected_prompts()
            .iter()
            .any(|id| id != BOT_INSTRUCTIONS_ID)
    }

    pub(super) fn prompt_summary_count(&self) -> usize {
        self.ordered_choices()
            .iter()
            .filter(|id| !id.starts_with("agena.system."))
            .count()
    }

    pub(super) fn select_personal_instructions(&mut self) {
        let order = self.prompt_order();
        self.prompts.selected = order
            .iter()
            .position(|id| id == BOT_INSTRUCTIONS_ID)
            .unwrap_or_else(|| self.prompts.selected.min(order.len().saturating_sub(1)));
    }
}

impl App {
    pub(super) fn handle_bot_prompt_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc && self.bot.task.is_some() {
            self.bot.cancel();
            self.current_route = Route::BotEntry(self.bot.request_id);
            self.bot.view = BotView::Form;
            return;
        }
        if self.bot.task.is_some() {
            return;
        }
        let Some(form) = &mut self.bot.form else {
            return;
        };
        if form.prompts.detail {
            let scroll = form.prompts.scroll.get();
            match key.code {
                KeyCode::Esc | KeyCode::Enter => form.prompts.detail = false,
                KeyCode::Up => form.prompts.scroll.set(scroll.saturating_sub(1)),
                KeyCode::Down => form.prompts.scroll.set(scroll.saturating_add(1)),
                KeyCode::PageUp => form.prompts.scroll.set(scroll.saturating_sub(10)),
                KeyCode::PageDown => form.prompts.scroll.set(scroll.saturating_add(10)),
                KeyCode::Home => form.prompts.scroll.set(0),
                KeyCode::End => form.prompts.scroll.set(u16::MAX),
                _ => {}
            }
            return;
        }
        if form.prompts.library {
            let library = form.prompt_library();
            let count = library.len();
            match key.code {
                KeyCode::Esc => {
                    form.prompts.library = false;
                    form.prompts.selected = 0;
                }
                KeyCode::Enter => {
                    if let Some(prompt) = library.get(form.prompts.selected) {
                        let mut order = form.ordered_choices();
                        order.push(prompt.id.to_string());
                        form.set_prompt_order(order);
                        form.prompts.selected = form.prompt_order().len().saturating_sub(1);
                        form.prompts.library = false;
                    }
                }
                code => move_list_selection(code, &mut form.prompts.selected, count),
            }
            return;
        }
        let mut order = form.prompt_order();
        let selected = form.prompts.selected;
        let current = order.get(selected).map(String::as_str);
        match key.code {
            KeyCode::Esc => {
                self.bot.view = BotView::Form;
                self.bot.error = None;
            }
            KeyCode::Char('o') => {
                form.prompts.library = true;
                form.prompts.selected = 0;
            }
            KeyCode::Char('a') => {
                form.edit = Some(Editor::from_text(form.persona.instructions.clone()));
            }
            KeyCode::Enter if current == Some(BOT_INSTRUCTIONS_ID) => {
                form.edit = Some(Editor::from_text(form.persona.instructions.clone()));
            }
            KeyCode::Enter if current.is_some() => {
                form.prompts.detail = true;
                form.prompts.scroll.set(0);
            }
            KeyCode::Char('[' | ']')
                if form.prompts.catalog.is_some() && selected < order.len() =>
            {
                let delta = if key.code == KeyCode::Char('[') {
                    -1
                } else {
                    1
                };
                let other = selected.saturating_add_signed(delta);
                if other < order.len() && selected != other {
                    order.swap(selected, other);
                    form.set_prompt_order(order);
                    form.prompts.selected = other;
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(id) = current.filter(|id| !id.starts_with("agena.system.")) {
                    if id == BOT_INSTRUCTIONS_ID {
                        form.persona.instructions.clear();
                    }
                    form.set_prompt_order(
                        form.selected_prompts()
                            .iter()
                            .filter(|entry| entry.as_str() != id)
                            .cloned()
                            .collect(),
                    );
                    form.prompts.selected =
                        selected.min(form.prompt_order().len().saturating_sub(1));
                }
            }
            KeyCode::Char('p') if form.has_shared_order() => {
                if let Some(selection) = &mut form.persona.prompts {
                    selection.publish_to_channels = !selection.publish_to_channels;
                    form.request_key = Uuid::new_v4().to_string();
                }
            }
            KeyCode::Char('s') => self.save_bot_form(),
            code => move_list_selection(code, &mut form.prompts.selected, order.len()),
        }
    }

    pub(super) fn render_bot_prompts(&self, frame: &mut Frame, area: Rect, form: &BotForm) {
        if area.height < 2 {
            return;
        }
        let editor = &form.prompts;
        let selected = editor.selected;
        let rows: Vec<_> = if editor.library {
            form.prompt_library()
                .into_iter()
                .enumerate()
                .map(|(index, prompt)| {
                    PromptRow::custom(prompt, &self.i18n, index == selected, |id| {
                        editor.catalog.as_ref()?.library.get(id.as_str())
                    })
                })
                .collect()
        } else {
            form.prompt_order()
                .into_iter()
                .enumerate()
                .map(|(index, id)| {
                    let mut row = self.bot_prompt_row(form, id, index == selected);
                    row.position = Some(index + 1);
                    row
                })
                .collect()
        };
        let row = rows.get(selected);
        if editor.detail
            && let Some(row) = row
        {
            render_detail(
                frame,
                area.inner(Margin::new(1, 0)),
                row,
                Some(&editor.scroll),
                &self.i18n,
            );
            return;
        }
        let [heading, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        let mut title = self.i18n.text(if editor.library {
            "prompt-studio-library"
        } else {
            "prompt-studio-order"
        });
        if !editor.library && form.has_shared_order() {
            let publish = form
                .persona
                .prompts
                .as_ref()
                .is_some_and(|selection| selection.publish_to_channels);
            title = format!(
                "{title}  ·  {}",
                self.i18n.text(if publish {
                    "bot-prompts-channel-on"
                } else {
                    "bot-prompts-channel-off"
                })
            );
        }
        frame.render_widget(Line::styled(title, theme::modal_border_style()), heading);
        if rows.is_empty() {
            let message = self.i18n.text(if self.bot.task.is_some() {
                "bot-prompts-loading"
            } else {
                "prompt-studio-empty"
            });
            frame.render_widget(
                Paragraph::new(message)
                    .wrap(Wrap { trim: true })
                    .style(theme::muted_style()),
                body,
            );
            return;
        }
        let Some(sidebar) = row.filter(|_| body.width >= 73) else {
            render_rows(frame, body, &rows, selected);
            return;
        };
        let [list, detail] =
            Layout::horizontal([Constraint::Percentage(45), Constraint::Fill(1)]).areas(body);
        let border = Block::new()
            .borders(Borders::LEFT)
            .border_style(theme::muted_style());
        let inner = border.inner(detail).inner(Margin::new(1, 0));
        frame.render_widget(border, detail);
        render_rows(frame, list, &rows, selected);
        render_detail(frame, inner, sidebar, None, &self.i18n);
    }

    fn bot_prompt_row(&self, form: &BotForm, id: String, read_body: bool) -> PromptRow {
        if let Some(catalog) = &form.prompts.catalog {
            if let Some(prompt) = catalog
                .builtins
                .iter()
                .find(|prompt| prompt.id.as_str() == id)
            {
                return PromptRow::builtin(prompt, &self.i18n, read_body);
            }
            if let Some(prompt) = catalog.library.get(&id) {
                return PromptRow::custom(prompt, &self.i18n, read_body, |id| {
                    catalog.library.get(id.as_str())
                });
            }
        }
        let personal = id == BOT_INSTRUCTIONS_ID;
        PromptRow {
            title: if personal {
                self.i18n.text("bot-instructions")
            } else {
                id.clone()
            },
            description: self.i18n.text(if personal {
                "prompt-studio-custom"
            } else {
                "prompt-studio-external"
            }),
            excerpt: if personal {
                prompt_excerpt(&form.persona.instructions, false)
            } else {
                String::new()
            },
            body: if personal && read_body {
                crate::sanitize_terminal_text(&form.persona.instructions)
            } else {
                self.i18n.text("prompt-studio-external-detail")
            },
            position: None,
            builtin: id.starts_with("agena.system."),
        }
    }
}
